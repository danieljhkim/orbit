//! Append-only log + aggregation for the `duel` evaluation workflow.
//!
//! `.orbit/state/scoreboard/duel.json` is the single source of truth for the duel
//! evaluation harness. It is a flat file with the shape:
//!
//! ```json
//! { "schema_version": 1, "runs": [ <DuelRun>, ... ] }
//! ```
//!
//! **Design principle:** runs are append-only. Aggregates (per-role/per-agent
//! averages, merge rates, segmented views by `task_class.scope` or
//! `task_class.ambiguity`) are computed on demand from `&[DuelRun]` — nothing
//! is precomputed. This eliminates drift bugs, allows the scoring rubric to
//! evolve without schema migrations, and lets new slicing dimensions be added
//! without rewriting history. Computation is trivial at expected volumes.
//!
//! This module has three public surfaces:
//! - [`append_run`] — writes one run entry, atomically rewriting the file.
//! - [`load_runs`] — reads all run entries back.
//! - [`aggregate`] — pure function over `&[DuelRun]` returning a report.
//! - [`derive_task_scope`] — classifies a diff into `TaskScope`; lives here
//!   because `record_duel_scores` builds the `TaskClass` from git at record
//!   time and the CLI never needs to touch git.

// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use orbit_common::types::{
    Ambiguity, Decision, DuelRun, OrbitError, TaskScope, Verdict, all_agent_families,
};
use serde::{Deserialize, Serialize};

use orbit_common::utility::fs::{
    atomic_write_text_volatile as write_atomic, with_exclusive_file_lock,
};

const SCOREBOARD_FILENAME: &str = "duel.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;

/// On-disk envelope for the scoreboard file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct DuelScoreboardFile {
    schema_version: u32,
    #[serde(default)]
    runs: Vec<DuelRun>,
}

impl Default for DuelScoreboardFile {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            runs: Vec::new(),
        }
    }
}

// ============================================================================
// Append + load
// ============================================================================

/// Append a single [`DuelRun`] to `scoreboard_dir/duel.json`, creating the
/// file on first use. Uses the shared atomic-write helper so a crash during
/// the rewrite cannot corrupt earlier entries.
pub fn append_run(scoreboard_dir: &Path, run: &DuelRun) -> Result<(), OrbitError> {
    let path = scoreboard_dir.join(SCOREBOARD_FILENAME);
    with_exclusive_file_lock(&path, "duel scoreboard", || {
        let mut file = load_scoreboard_file(&path)?;
        file.runs.push(run.clone());

        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| OrbitError::Io(format!("serialize duel.json: {e}")))?;
        write_atomic(&path, &format!("{json}\n")).map_err(Into::into)
    })
}

/// Load every run entry from `scoreboard_dir/duel.json`. Returns an empty
/// vector if the file does not yet exist.
pub fn load_runs(scoreboard_dir: &Path) -> Result<Vec<DuelRun>, OrbitError> {
    let path = scoreboard_dir.join(SCOREBOARD_FILENAME);
    Ok(load_scoreboard_file(&path)?.runs)
}

fn load_scoreboard_file(path: &Path) -> Result<DuelScoreboardFile, OrbitError> {
    if !path.exists() {
        return Ok(DuelScoreboardFile::default());
    }
    let content =
        fs::read_to_string(path).map_err(|e| OrbitError::Io(format!("read duel.json: {e}")))?;
    if content.trim().is_empty() {
        return Ok(DuelScoreboardFile::default());
    }
    serde_json::from_str(&content).map_err(|e| OrbitError::Io(format!("parse duel.json: {e}")))
}

// ============================================================================
// Task scope derivation (runtime crate discovery, no hardcoded list)
// ============================================================================

/// Derive a [`TaskScope`] for a duel run by diffing `head_ref` against
/// `base_ref` inside `repo_root` and walking the file list against the
/// runtime-discovered crate directories under `orbit/`.
///
/// If `git diff` fails for any reason (missing base ref, repository in a
/// degenerate state, ...) the caller receives a descriptive error rather
/// than a silent default — scope is a signal we do not want to fabricate.
pub fn derive_task_scope(
    repo_root: &Path,
    base_ref: &str,
    head_ref: &str,
) -> Result<TaskScope, OrbitError> {
    let output = Command::new("git")
        .arg("diff")
        .arg("--name-only")
        .arg(format!("{base_ref}...{head_ref}"))
        .current_dir(repo_root)
        .output()
        .map_err(|e| OrbitError::Io(format!("spawn git diff: {e}")))?;

    if !output.status.success() {
        return Err(OrbitError::Io(format!(
            "git diff {base_ref}...{head_ref} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim(),
        )));
    }

    let files: Vec<&str> = std::str::from_utf8(&output.stdout)
        .map_err(|e| OrbitError::Io(format!("git diff output not utf-8: {e}")))?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    let crates = discover_crate_dirs(repo_root);
    Ok(classify_scope(&files, &crates))
}

/// Walk `repo_root/crates/` and return the set of crate directory names.
/// A crate directory is any immediate child of `crates/` that contains a
/// `Cargo.toml`. Returned as a `Vec<String>` sorted for determinism.
///
/// This is discovered at runtime precisely so adding a new crate does NOT
/// require touching this module.
fn discover_crate_dirs(repo_root: &Path) -> Vec<String> {
    let crates_dir = repo_root.join("crates");
    let Ok(entries) = fs::read_dir(&crates_dir) else {
        return Vec::new();
    };
    let mut crates: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }
            if !path.join("Cargo.toml").exists() {
                return None;
            }
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        })
        .collect();
    crates.sort();
    crates
}

/// Classify a set of changed paths into [`TaskScope`].
///
/// Rules (checked in this order):
/// - Empty diff → `Other`.
/// - Exactly one changed file → `SingleFile`.
/// - Two or more files, all bucketed into the same real crate → `MultiFile`.
/// - Two or more files, all bucketed into `Other` (non-crate) → `Other`.
/// - Any mixture of buckets (or 2+ real crates) → `CrossCrate`.
///
/// Non-crate files (`README.md`, `scripts/`, `Makefile`, ...) are bucketed
/// as the synthetic `"other"` key so a README-only diff surfaces as `Other`
/// and a README + crate mix surfaces as `CrossCrate`.
fn classify_scope(files: &[&str], crates: &[String]) -> TaskScope {
    match files.len() {
        0 => TaskScope::Other,
        1 => {
            // Even single-file changes record a scope; callers that want
            // bucket info for aggregation can recompute with more than one.
            TaskScope::SingleFile
        }
        _ => {
            let buckets: BTreeSet<String> =
                files.iter().map(|f| bucket_for_path(f, crates)).collect();
            if buckets.len() == 1 {
                let bucket = buckets.iter().next().expect("len == 1");
                if bucket == "other" {
                    TaskScope::Other
                } else {
                    TaskScope::MultiFile
                }
            } else {
                TaskScope::CrossCrate
            }
        }
    }
}

/// Map a repo-relative path to its bucket name:
/// - `crates/<crate>/...` where `<crate>` is in `crates` → `<crate>`
/// - anything else → `"other"`
///
/// Exposed at module scope for focused unit testing.
fn bucket_for_path(path: &str, crates: &[String]) -> String {
    if let Some(rest) = path.strip_prefix("crates/")
        && let Some(first) = rest.split('/').next()
        && crates.iter().any(|c| c == first)
    {
        return first.to_string();
    }
    "other".to_string()
}

// ============================================================================
// Aggregation (pure function over runs)
// ============================================================================

/// Role axis for aggregation filtering. Mirrors the `--role` CLI flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleAxis {
    Implementer,
    Reviewer,
    Arbiter,
}

/// Segmentation dimension for the `--by` CLI flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SegmentBy {
    #[default]
    None,
    Scope,
    Ambiguity,
}

/// Filter applied by [`aggregate`] before reducing.
#[derive(Debug, Clone, Copy, Default)]
pub struct AggregateFilter {
    /// Restrict to a single role. `None` emits all three.
    pub role: Option<RoleAxis>,
    /// Segment the output along a secondary axis.
    pub segment_by: SegmentBy,
}

/// One row of the aggregated scoreboard view.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AggregateRow {
    /// Segment key; empty string when `segment_by == SegmentBy::None`.
    pub segment: String,
    pub role: &'static str,
    pub agent: String,
    pub model: String,
    pub runs: u32,
    pub avg_score: f64,
    pub merge_rate: f64,
    pub avg_fix_iterations: f64,
    pub avg_wall_seconds: f64,
}

/// Aggregation result. Rows are sorted by (segment, role, agent, model) for
/// deterministic table rendering.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Aggregates {
    pub rows: Vec<AggregateRow>,
}

/// Reduce `runs` into an [`Aggregates`] report.
///
/// This is the only reduction function in the module; the CLI command is a
/// thin wrapper that reads runs via [`load_runs`] and calls this with the
/// flags the user passed. Tests exercise this directly without touching the
/// filesystem.
pub fn aggregate(runs: &[DuelRun], filter: AggregateFilter) -> Aggregates {
    // Bucket key: (segment, role, agent, model)
    #[derive(Default)]
    struct Bucket {
        runs: u32,
        score_sum: f64,
        merged: u32,
        fix_iter_sum: u64,
        wall_sum: u64,
    }

    let mut buckets: BTreeMap<(String, &'static str, String, String), Bucket> = BTreeMap::new();

    let roles_to_emit: &[RoleAxis] = match filter.role {
        Some(RoleAxis::Implementer) => &[RoleAxis::Implementer],
        Some(RoleAxis::Reviewer) => &[RoleAxis::Reviewer],
        Some(RoleAxis::Arbiter) => &[RoleAxis::Arbiter],
        None => &[RoleAxis::Implementer, RoleAxis::Reviewer, RoleAxis::Arbiter],
    };

    let seed_segments: Vec<String> = if filter.segment_by == SegmentBy::None {
        vec![String::new()]
    } else {
        let mut segments: BTreeSet<String> = runs
            .iter()
            .map(|run| segment_key_for(run, filter.segment_by))
            .collect();
        if segments.is_empty() {
            segments.insert(String::new());
        }
        segments.into_iter().collect()
    };

    for segment_key in seed_segments {
        for role in roles_to_emit {
            let role_name = match role {
                RoleAxis::Implementer => "implementer",
                RoleAxis::Reviewer => "reviewer",
                RoleAxis::Arbiter => "arbiter",
            };
            for family in all_agent_families() {
                buckets
                    .entry((
                        segment_key.clone(),
                        role_name,
                        family.to_string(),
                        default_orchestrator_model(family),
                    ))
                    .or_default();
            }
        }
    }

    for run in runs {
        let segment_key = segment_key_for(run, filter.segment_by);
        for role in roles_to_emit {
            let (role_name, assignment, score) = match role {
                RoleAxis::Implementer => (
                    "implementer",
                    &run.roles.implementer,
                    run.scores.implementer_score as f64,
                ),
                RoleAxis::Reviewer => (
                    "reviewer",
                    &run.roles.reviewer,
                    run.scores.reviewer_score as f64,
                ),
                RoleAxis::Arbiter => {
                    // Arbiter self-assessment is skipped in v1 per the task
                    // spec — there is no arbiter score. Only the run count
                    // and cost fields are meaningful for this role.
                    ("arbiter", &run.roles.arbiter, 0.0)
                }
            };
            let key = (
                segment_key.clone(),
                role_name,
                assignment.agent.clone(),
                assignment.model.clone(),
            );
            let bucket = buckets.entry(key).or_default();
            bucket.runs += 1;
            bucket.score_sum += score;
            if run.outcome.merged {
                bucket.merged += 1;
            }
            bucket.fix_iter_sum += run.outcome.fix_loop_iterations as u64;
            bucket.wall_sum += run.cost.wall_clock_seconds;
        }
    }

    let rows = buckets
        .into_iter()
        .map(|((segment, role, agent, model), b)| {
            let runs = b.runs.max(1) as f64;
            AggregateRow {
                segment,
                role,
                agent,
                model,
                runs: b.runs,
                avg_score: b.score_sum / runs,
                merge_rate: b.merged as f64 / runs,
                avg_fix_iterations: b.fix_iter_sum as f64 / runs,
                avg_wall_seconds: b.wall_sum as f64 / runs,
            }
        })
        .collect();

    Aggregates { rows }
}

fn default_orchestrator_model(family: &str) -> String {
    family.to_string()
}

fn segment_key_for(run: &DuelRun, axis: SegmentBy) -> String {
    match axis {
        SegmentBy::None => String::new(),
        SegmentBy::Scope => match run.task_class.scope {
            TaskScope::SingleFile => "single_file".into(),
            TaskScope::MultiFile => "multi_file".into(),
            TaskScope::CrossCrate => "cross_crate".into(),
            TaskScope::Other => "other".into(),
        },
        SegmentBy::Ambiguity => match run.task_class.ambiguity {
            Some(Ambiguity::WellSpecified) => "well_specified".into(),
            Some(Ambiguity::NeedsJudgment) => "needs_judgment".into(),
            Some(Ambiguity::Exploratory) => "exploratory".into(),
            None => "unknown".into(),
        },
    }
}

// ============================================================================
// Reviewer-stats helpers (used by record_duel_scores when building DuelRun)
// ============================================================================

/// Tally a list of per-comment verdicts into the four counts plus precision
/// and arbiter_override_rate. Split out so the executor does not duplicate
/// the math and the edge cases (zero comments) stay tested in one place.
pub fn tally_reviewer_stats(verdicts: &[Verdict]) -> ReviewerTally {
    let total = verdicts.len() as u32;
    let mut valid = 0u32;
    let mut invalid = 0u32;
    let mut out_of_scope = 0u32;
    let mut nitpick = 0u32;
    for v in verdicts {
        match v {
            Verdict::Valid => valid += 1,
            Verdict::Invalid => invalid += 1,
            Verdict::OutOfScope => out_of_scope += 1,
            Verdict::Nitpick => nitpick += 1,
        }
    }
    let (precision, override_rate) = if total == 0 {
        (0.0, 0.0)
    } else {
        (
            valid as f64 / total as f64,
            (invalid + out_of_scope + nitpick) as f64 / total as f64,
        )
    };
    ReviewerTally {
        total_comments: total,
        valid,
        invalid,
        out_of_scope,
        nitpick,
        precision,
        arbiter_override_rate: override_rate,
    }
}

/// Output of [`tally_reviewer_stats`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReviewerTally {
    pub total_comments: u32,
    pub valid: u32,
    pub invalid: u32,
    pub out_of_scope: u32,
    pub nitpick: u32,
    pub precision: f64,
    pub arbiter_override_rate: f64,
}

// ============================================================================
// Intentional re-export: the candidate agent family set, so downstream
// callers can reach it via `orbit_store::duel_scoreboard` without a second
// `orbit-types` import. This is cosmetic — `orbit_common::types::all_agent_families`
// is still the source of truth.
// ============================================================================
pub use orbit_common::types::all_agent_families as known_agent_families;

// Silence an unused-import lint when the module is consumed by callers that
// do not need the Decision re-export directly.
#[allow(dead_code)]
fn _ensure_decision_in_scope(_: Decision) {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[cfg(test)]
mod tests;
