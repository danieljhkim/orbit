use clap::{Args, Subcommand};
use orbit_core::command::job_run::JobRunListParams;
use orbit_core::knowledge_stats::{KnowledgeStatsSummary, aggregate};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Inspect knowledge-pack usage and savings metrics")]
pub struct KnowledgeCommand {
    #[command(subcommand)]
    pub command: KnowledgeSubcommand,
}

impl Execute for KnowledgeCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum KnowledgeSubcommand {
    /// Aggregate implement_change knowledge-pack metrics from job runs
    Stats(KnowledgeStatsArgs),
}

impl Execute for KnowledgeSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            KnowledgeSubcommand::Stats(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct KnowledgeStatsArgs {
    /// Limit the number of most-recent job runs considered
    #[arg(long)]
    pub limit: Option<usize>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for KnowledgeStatsArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let summary = load_summary(runtime, self.limit)?;

        if self.json {
            return crate::output::json::print_pretty(
                &serde_json::to_value(&summary).map_err(|e| OrbitError::Store(e.to_string()))?,
            );
        }

        print_summary(&summary);
        Ok(())
    }
}

fn load_summary(
    runtime: &OrbitRuntime,
    limit: Option<usize>,
) -> Result<KnowledgeStatsSummary, OrbitError> {
    let runs = runtime.list_job_runs(JobRunListParams {
        limit,
        ..Default::default()
    })?;
    Ok(aggregate(&runs))
}

fn print_summary(summary: &KnowledgeStatsSummary) {
    if summary.total_runs == 0 {
        println!("No knowledge usage metrics found.");
        return;
    }

    println!("orbit knowledge stats");
    println!();
    println!(
        "Runs:        {} total | {} with pack | {} fallback ({:.1}% fallback rate)",
        summary.total_runs,
        summary.pack_runs,
        summary.fallback_runs,
        summary.fallback_rate * 100.0
    );
    println!();

    if let Some(compression) = &summary.compression {
        println!("Compression (tokenized, cl100k_base):");
        println!("  Mean:      {:.1}x", compression.mean);
        println!("  p50:       {:.1}x", compression.p50);
        println!("  p90:       {:.1}x", compression.p90);
        println!("  Min:       {:.1}x", compression.min);
        println!();
    }

    println!("Double-read guard:");
    println!(
        "  Mean rate: {:.2} ({:.0}% of baseline tokens re-read via fs.read)",
        summary.double_read.mean_rate,
        summary.double_read.mean_rate * 100.0
    );
    println!(
        "  Runs >50%: {} / {} (flag for investigation)",
        summary.double_read.runs_over_fifty_percent, summary.double_read.measured_runs
    );
    println!();

    println!("Total LLM input tokens per activity:");
    println!(
        "  With pack:    avg {:.0} tokens",
        summary.total_llm_input_tokens.with_pack_avg
    );
    println!(
        "  Without pack: avg {:.0} tokens",
        summary.total_llm_input_tokens.without_pack_avg
    );
    if let Some(savings) = summary.total_llm_input_tokens.estimated_savings {
        println!("  Estimated savings: {:.0}%", savings * 100.0);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::Utc;
    use orbit_core::OrbitRuntime;
    use orbit_types::{JobRun, JobRunState, KnowledgeRunMetrics};
    use tempfile::tempdir;

    use super::load_summary;

    fn seed_run(root: &std::path::Path, run_id: &str, metrics: KnowledgeRunMetrics) {
        let run_dir = root.join("jobs/runs/knowledge-job").join(run_id);
        fs::create_dir_all(&run_dir).expect("run dir");
        let run = JobRun {
            run_id: run_id.to_string(),
            job_id: "knowledge-job".to_string(),
            attempt: 1,
            state: JobRunState::Success,
            scheduled_at: Utc::now(),
            started_at: None,
            finished_at: None,
            duration_ms: None,
            created_at: Utc::now(),
            pid: None,
            pid_start_time: None,
            input: None,
            retry_source_run_id: None,
            knowledge_metrics: Some(metrics),
            steps: vec![],
        };
        let yaml = serde_yaml::to_string(&serde_json::json!({
            "schemaVersion": 1,
            "run": run,
        }))
        .expect("serialize");
        fs::write(run_dir.join("jrun.yaml"), yaml).expect("write run");
    }

    #[test]
    fn load_summary_reads_recent_job_run_fixture() {
        let repo = tempdir().expect("tempdir");
        let orbit_root = repo.path().join(".orbit");
        fs::create_dir_all(&orbit_root).expect("orbit root");

        seed_run(
            &orbit_root,
            "r1",
            KnowledgeRunMetrics {
                raw_read_token_baseline: 100,
                knowledge_pack_tokens: Some(20),
                compression_ratio: Some(5.0),
                actual_fs_read_tokens_during_run: 10,
                double_read_rate: Some(0.1),
                knowledge_pack_used: true,
                knowledge_pack_unresolved_count: 0,
                total_llm_input_tokens: 400,
            },
        );
        seed_run(
            &orbit_root,
            "r2",
            KnowledgeRunMetrics {
                raw_read_token_baseline: 120,
                knowledge_pack_tokens: Some(30),
                compression_ratio: Some(4.0),
                actual_fs_read_tokens_during_run: 12,
                double_read_rate: Some(0.1),
                knowledge_pack_used: true,
                knowledge_pack_unresolved_count: 1,
                total_llm_input_tokens: 500,
            },
        );
        seed_run(
            &orbit_root,
            "r3",
            KnowledgeRunMetrics {
                raw_read_token_baseline: 90,
                knowledge_pack_tokens: Some(45),
                compression_ratio: Some(2.0),
                actual_fs_read_tokens_during_run: 63,
                double_read_rate: Some(0.7),
                knowledge_pack_used: true,
                knowledge_pack_unresolved_count: 2,
                total_llm_input_tokens: 700,
            },
        );
        seed_run(
            &orbit_root,
            "r4",
            KnowledgeRunMetrics {
                raw_read_token_baseline: 80,
                knowledge_pack_tokens: None,
                compression_ratio: None,
                actual_fs_read_tokens_during_run: 16,
                double_read_rate: Some(0.2),
                knowledge_pack_used: false,
                knowledge_pack_unresolved_count: 0,
                total_llm_input_tokens: 900,
            },
        );
        seed_run(
            &orbit_root,
            "r5",
            KnowledgeRunMetrics {
                raw_read_token_baseline: 70,
                knowledge_pack_tokens: None,
                compression_ratio: None,
                actual_fs_read_tokens_during_run: 21,
                double_read_rate: Some(0.3),
                knowledge_pack_used: false,
                knowledge_pack_unresolved_count: 0,
                total_llm_input_tokens: 1000,
            },
        );

        let runtime = OrbitRuntime::from_roots(&orbit_root, &orbit_root).expect("runtime");
        let summary = load_summary(&runtime, None).expect("summary");

        assert_eq!(summary.total_runs, 5);
        assert_eq!(summary.pack_runs, 3);
        assert_eq!(summary.fallback_runs, 2);
        assert!(summary.compression.is_some());
        assert_eq!(summary.double_read.runs_over_fifty_percent, 1);
        assert_eq!(summary.double_read.measured_runs, 5);
        assert!(summary.total_llm_input_tokens.estimated_savings.is_some());
    }
}
