use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::error::KnowledgeError;
use crate::io::write_text_atomic_durable;
use crate::pipeline::context::BuildConfig;
use crate::pipeline::run_build;

pub const SCOREBOARD_CAP: usize = 200;

#[derive(Debug, Clone)]
pub struct GraphBenchOptions {
    pub workspace: PathBuf,
    pub knowledge_dir: PathBuf,
    pub scoreboard_path: PathBuf,
}

impl GraphBenchOptions {
    pub fn from_workspace(workspace: impl Into<PathBuf>) -> Self {
        let workspace = workspace.into();
        Self {
            knowledge_dir: workspace.join(".orbit/knowledge"),
            scoreboard_path: workspace.join(".orbit/state/scoreboard/graph_bench.json"),
            workspace,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchScenario {
    ColdBuild,
    WarmIncrementalNoop,
}

impl BenchScenario {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ColdBuild => "cold_build",
            Self::WarmIncrementalNoop => "warm_incremental_noop",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "cold_build" => Some(Self::ColdBuild),
            "warm_incremental_noop" => Some(Self::WarmIncrementalNoop),
            _ => None,
        }
    }

    fn incremental(self) -> bool {
        matches!(self, Self::WarmIncrementalNoop)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioMetrics {
    pub wall_time_ms: u64,
    pub peak_rss_kib: Option<u64>,
    pub file_count: usize,
    pub leaf_count: usize,
    pub dir_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphBenchScenarios {
    pub cold_build: ScenarioMetrics,
    pub warm_incremental_noop: ScenarioMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphBenchRecord {
    pub timestamp: String,
    pub git_sha: String,
    pub hostname: String,
    pub logical_core_count: usize,
    pub scenarios: GraphBenchScenarios,
}

#[derive(Debug, Clone)]
pub struct GraphBenchOutcome {
    pub record: GraphBenchRecord,
    pub previous: Option<GraphBenchRecord>,
    pub summary: String,
}

/// Runs both scenarios in the **current process**.
///
/// `peak_rss_kib` reflects `getrusage(RUSAGE_SELF)`, which is monotonic across
/// the process lifetime — the warm scenario's RSS therefore includes the cold
/// scenario's high-water mark. For accurate per-scenario RSS, use
/// [`run_benchmark_with_child_process`]. This in-process variant is useful for
/// embedded callers and tests that do not care about RSS isolation.
pub fn run_benchmark_in_process(
    options: &GraphBenchOptions,
) -> Result<GraphBenchOutcome, KnowledgeError> {
    run_benchmark_with_runner(options, |scenario| run_single_scenario(options, scenario))
}

pub fn run_benchmark_with_child_process(
    options: &GraphBenchOptions,
    executable: &Path,
) -> Result<GraphBenchOutcome, KnowledgeError> {
    let scratch_dir = temporary_child_dir(&options.scoreboard_path);
    fs::create_dir_all(&scratch_dir)
        .map_err(|error| KnowledgeError::io(format!("mkdir graph bench scratch dir: {error}")))?;

    let result = run_benchmark_with_runner(options, |scenario| {
        run_child_scenario(options, executable, &scratch_dir, scenario)
    });

    let cleanup_result = fs::remove_dir_all(&scratch_dir);
    match (result, cleanup_result) {
        (Ok(outcome), Ok(())) => Ok(outcome),
        (Ok(outcome), Err(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(outcome),
        (Ok(_), Err(error)) => Err(KnowledgeError::io(format!(
            "remove graph bench scratch dir: {error}"
        ))),
        (Err(error), _) => Err(error),
    }
}

fn run_benchmark_with_runner<F>(
    options: &GraphBenchOptions,
    mut runner: F,
) -> Result<GraphBenchOutcome, KnowledgeError>
where
    F: FnMut(BenchScenario) -> Result<ScenarioMetrics, KnowledgeError>,
{
    if options.knowledge_dir.exists() {
        fs::remove_dir_all(&options.knowledge_dir).map_err(|error| {
            KnowledgeError::io(format!(
                "clean knowledge dir {}: {error}",
                options.knowledge_dir.display()
            ))
        })?;
    }

    let cold_build = runner(BenchScenario::ColdBuild)?;
    let warm_incremental_noop = runner(BenchScenario::WarmIncrementalNoop)?;
    let record = GraphBenchRecord {
        timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        git_sha: git_sha(&options.workspace),
        hostname: hostname(),
        logical_core_count: logical_core_count(),
        scenarios: GraphBenchScenarios {
            cold_build,
            warm_incremental_noop,
        },
    };
    let previous = append_scoreboard(&options.scoreboard_path, record.clone())?;
    let summary = format_summary(&record, previous.as_ref());
    Ok(GraphBenchOutcome {
        record,
        previous,
        summary,
    })
}

pub fn run_single_scenario(
    options: &GraphBenchOptions,
    scenario: BenchScenario,
) -> Result<ScenarioMetrics, KnowledgeError> {
    let started = Instant::now();
    let ctx = run_build(BuildConfig {
        repo_path: options.workspace.clone(),
        output_dir: options.knowledge_dir.clone(),
        incremental: scenario.incremental(),
        ref_name: None,
    })?;
    let wall_time_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

    Ok(ScenarioMetrics {
        wall_time_ms,
        peak_rss_kib: peak_rss_kib(),
        file_count: ctx.graph.files.len(),
        leaf_count: ctx.graph.leaves.len(),
        dir_count: ctx.graph.dirs.len(),
    })
}

pub fn write_child_metrics(path: &Path, metrics: &ScenarioMetrics) -> Result<(), KnowledgeError> {
    let json = serde_json::to_string_pretty(metrics)
        .map_err(|error| KnowledgeError::invalid_data(format!("serialize metrics: {error}")))?;
    write_text_atomic_durable(path, &format!("{json}\n"))
        .map_err(|error| KnowledgeError::io(format!("write child metrics: {error}")))
}

fn run_child_scenario(
    options: &GraphBenchOptions,
    executable: &Path,
    scratch_dir: &Path,
    scenario: BenchScenario,
) -> Result<ScenarioMetrics, KnowledgeError> {
    let output_path = scratch_dir.join(format!("{}.json", scenario.as_str()));
    let status = Command::new(executable)
        .arg("--workspace")
        .arg(&options.workspace)
        .arg("--knowledge-dir")
        .arg(&options.knowledge_dir)
        .arg("--scoreboard")
        .arg(&options.scoreboard_path)
        .arg("--child-scenario")
        .arg(scenario.as_str())
        .arg("--child-output")
        .arg(&output_path)
        .status()
        .map_err(|error| KnowledgeError::io(format!("spawn graph bench child: {error}")))?;

    if !status.success() {
        return Err(KnowledgeError::knowledge_unavailable(format!(
            "graph bench child {} exited with {status}",
            scenario.as_str()
        )));
    }

    let raw = fs::read_to_string(&output_path)
        .map_err(|error| KnowledgeError::io(format!("read child metrics: {error}")))?;
    serde_json::from_str(&raw)
        .map_err(|error| KnowledgeError::invalid_data(format!("parse child metrics: {error}")))
}

pub fn append_scoreboard(
    path: &Path,
    record: GraphBenchRecord,
) -> Result<Option<GraphBenchRecord>, KnowledgeError> {
    let mut records = load_scoreboard(path)?;
    let previous = records.last().cloned();
    records.push(record);
    if records.len() > SCOREBOARD_CAP {
        let prune_count = records.len() - SCOREBOARD_CAP;
        records.drain(0..prune_count);
    }

    let json = serde_json::to_string_pretty(&records)
        .map_err(|error| KnowledgeError::invalid_data(format!("serialize scoreboard: {error}")))?;
    write_text_atomic_durable(path, &format!("{json}\n")).map_err(|error| {
        KnowledgeError::io(format!("write graph benchmark scoreboard: {error}"))
    })?;

    Ok(previous)
}

/// pub(crate) for direct use by sibling test in tests/graph_bench.rs
/// (follows per-module sibling tests layout; previously visible only via nested child).
pub(crate) fn load_scoreboard(path: &Path) -> Result<Vec<GraphBenchRecord>, KnowledgeError> {
    if !path.is_file() {
        return Ok(Vec::new());
    }

    let raw = fs::read_to_string(path)
        .map_err(|error| KnowledgeError::io(format!("read graph benchmark scoreboard: {error}")))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&raw)
        .map_err(|error| KnowledgeError::invalid_data(format!("parse scoreboard: {error}")))
}

pub fn format_summary(record: &GraphBenchRecord, previous: Option<&GraphBenchRecord>) -> String {
    let mut lines = vec![
        "Graph build benchmark".to_string(),
        format!("timestamp: {}", record.timestamp),
        format!("git_sha: {}", record.git_sha),
        format!("hostname: {}", record.hostname),
        format!("logical_core_count: {}", record.logical_core_count),
        format!(
            "{}: {}",
            BenchScenario::ColdBuild.as_str(),
            format_metrics(
                &record.scenarios.cold_build,
                previous.map(|prior| &prior.scenarios.cold_build)
            )
        ),
        format!(
            "{}: {}",
            BenchScenario::WarmIncrementalNoop.as_str(),
            format_metrics(
                &record.scenarios.warm_incremental_noop,
                previous.map(|prior| &prior.scenarios.warm_incremental_noop)
            )
        ),
    ];
    lines.push(format!(
        "scoreboard: .orbit/state/scoreboard/graph_bench.json (cap {} records)",
        SCOREBOARD_CAP
    ));
    lines.join("\n")
}

fn format_metrics(current: &ScenarioMetrics, previous: Option<&ScenarioMetrics>) -> String {
    format!(
        "{}ms {}, rss {}, files {} {}, leaves {} {}, dirs {} {}",
        current.wall_time_ms,
        delta_or_baseline(
            previous.map(|metrics| metrics.wall_time_ms),
            current.wall_time_ms
        ),
        format_rss(
            current.peak_rss_kib,
            previous.and_then(|metrics| metrics.peak_rss_kib)
        ),
        current.file_count,
        delta_or_baseline(
            previous.map(|metrics| metrics.file_count as u64),
            current.file_count as u64,
        ),
        current.leaf_count,
        delta_or_baseline(
            previous.map(|metrics| metrics.leaf_count as u64),
            current.leaf_count as u64,
        ),
        current.dir_count,
        delta_or_baseline(
            previous.map(|metrics| metrics.dir_count as u64),
            current.dir_count as u64,
        ),
    )
}

fn format_rss(current: Option<u64>, previous: Option<u64>) -> String {
    match current {
        Some(value) => format!("{value} KiB {}", delta_or_baseline(previous, value)),
        None => "n/a".to_string(),
    }
}

fn delta_or_baseline(previous: Option<u64>, current: u64) -> String {
    match previous {
        Some(prior) if prior > 0 => {
            let prior = prior as f64;
            let current = current as f64;
            let pct = ((current - prior) / prior) * 100.0;
            format!("({pct:+.0}% vs last)")
        }
        Some(_) => "(n/a vs last)".to_string(),
        None => "(baseline)".to_string(),
    }
}

fn git_sha(repo: &Path) -> String {
    command_stdout(repo, "git", &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string())
}

fn command_stdout(current_dir: &Path, command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command)
        .args(args)
        .current_dir(current_dir)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn logical_core_count() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
}

fn hostname() -> String {
    unix_hostname()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(unix)]
fn unix_hostname() -> Option<String> {
    let mut buffer = [0u8; 256];
    let rc = unsafe { libc::gethostname(buffer.as_mut_ptr().cast::<libc::c_char>(), buffer.len()) };
    if rc != 0 {
        return None;
    }
    let len = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    Some(String::from_utf8_lossy(&buffer[..len]).to_string())
}

#[cfg(not(unix))]
fn unix_hostname() -> Option<String> {
    None
}

#[cfg(unix)]
fn peak_rss_kib() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if rc != 0 {
        return None;
    }

    let rss = unsafe { usage.assume_init().ru_maxrss };
    if rss < 0 {
        return None;
    }

    #[cfg(target_os = "macos")]
    {
        Some((rss as u64).saturating_add(1023) / 1024)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Some(rss as u64)
    }
}

#[cfg(not(unix))]
fn peak_rss_kib() -> Option<u64> {
    None
}

fn temporary_child_dir(scoreboard_path: &Path) -> PathBuf {
    let parent = scoreboard_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!(
        ".graph_bench_tmp_{}_{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}
