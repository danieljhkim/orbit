//! Append-only log + aggregation for the planning-duel workflow.
//!
//! `.orbit/state/scoreboard/duel_plan.json` stores one row per planning duel. Each
//! row captures both planner proposals, the arbiter's winner, and per-role
//! efficiency metrics. Aggregates are computed on demand from the raw rows so
//! the report stays deterministic and schema-light.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use orbit_common::types::{
    OrbitError, PlannerSlot, PlanningDuelRun, all_agent_families, resolve_agent_model_pair,
};
use serde::{Deserialize, Serialize};

use orbit_common::utility::fs::{
    atomic_write_text_volatile as write_atomic, with_exclusive_file_lock,
};

const SCOREBOARD_FILENAME: &str = "duel_plan.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;

/// On-disk envelope for the planning-duel scoreboard file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct PlanningDuelScoreboardFile {
    schema_version: u32,
    #[serde(default)]
    runs: Vec<PlanningDuelRun>,
}

impl Default for PlanningDuelScoreboardFile {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            runs: Vec::new(),
        }
    }
}

/// The three role axes that can be aggregated independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleAxis {
    PlannerA,
    PlannerB,
    Arbiter,
}

/// Filter applied by [`aggregate`] before reducing.
#[derive(Debug, Clone, Copy, Default)]
pub struct AggregateFilter {
    /// Restrict to a single role. `None` emits all three.
    pub role: Option<RoleAxis>,
}

/// One row of the aggregated planning-duel view.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AggregateRow {
    pub role: &'static str,
    pub agent: String,
    pub model: String,
    pub runs: u32,
    pub points: u32,
    pub avg_wall_seconds: f64,
    pub avg_tool_calls: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_token_total: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_byte_proxy_total: Option<f64>,
}

/// Aggregation result. Rows are sorted by role, agent, and model for
/// deterministic rendering.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Aggregates {
    pub rows: Vec<AggregateRow>,
}

#[derive(Default)]
struct Bucket {
    runs: u32,
    points: u32,
    wall_ms_sum: u128,
    tool_calls_sum: u128,
    token_sum: u128,
    token_count: u32,
    byte_proxy_sum: u128,
    byte_proxy_count: u32,
}

// ============================================================================
// Append + load
// ============================================================================

/// Append a single [`PlanningDuelRun`] to `scoreboard_dir/duel_plan.json`.
pub fn append_run(scoreboard_dir: &Path, run: &PlanningDuelRun) -> Result<(), OrbitError> {
    let path = scoreboard_dir.join(SCOREBOARD_FILENAME);
    with_exclusive_file_lock(&path, "planning duel scoreboard", || {
        let mut file = load_scoreboard_file(&path)?;
        file.runs.push(run.clone());

        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| OrbitError::Io(format!("serialize duel_plan.json: {e}")))?;
        write_atomic(&path, &format!("{json}\n")).map_err(Into::into)
    })
}

/// Load every run entry from `scoreboard_dir/duel_plan.json`.
pub fn load_runs(scoreboard_dir: &Path) -> Result<Vec<PlanningDuelRun>, OrbitError> {
    let path = scoreboard_dir.join(SCOREBOARD_FILENAME);
    Ok(load_scoreboard_file(&path)?.runs)
}

fn load_scoreboard_file(path: &Path) -> Result<PlanningDuelScoreboardFile, OrbitError> {
    if !path.exists() {
        return Ok(PlanningDuelScoreboardFile::default());
    }
    let content = fs::read_to_string(path)
        .map_err(|e| OrbitError::Io(format!("read duel_plan.json: {e}")))?;
    if content.trim().is_empty() {
        return Ok(PlanningDuelScoreboardFile::default());
    }
    serde_json::from_str(&content).map_err(|e| OrbitError::Io(format!("parse duel_plan.json: {e}")))
}

// ============================================================================
// Aggregation
// ============================================================================

/// Reduce `runs` into an [`Aggregates`] report.
pub fn aggregate(runs: &[PlanningDuelRun], filter: AggregateFilter) -> Aggregates {
    let mut buckets: BTreeMap<(String, &'static str, String, String), Bucket> = BTreeMap::new();

    let roles_to_emit: &[RoleAxis] = match filter.role {
        Some(RoleAxis::PlannerA) => &[RoleAxis::PlannerA],
        Some(RoleAxis::PlannerB) => &[RoleAxis::PlannerB],
        Some(RoleAxis::Arbiter) => &[RoleAxis::Arbiter],
        None => &[RoleAxis::PlannerA, RoleAxis::PlannerB, RoleAxis::Arbiter],
    };

    seed_zero_family_rows(&mut buckets, roles_to_emit);

    for run in runs {
        for role in roles_to_emit {
            let (role_name, assignment, metrics, points) = match role {
                RoleAxis::PlannerA => (
                    "planner_a",
                    &run.roles.planner_a,
                    &run.efficiency.planner_a,
                    if run.outcome.winner == PlannerSlot::PlannerA {
                        1
                    } else {
                        0
                    },
                ),
                RoleAxis::PlannerB => (
                    "planner_b",
                    &run.roles.planner_b,
                    &run.efficiency.planner_b,
                    if run.outcome.winner == PlannerSlot::PlannerB {
                        1
                    } else {
                        0
                    },
                ),
                RoleAxis::Arbiter => ("arbiter", &run.roles.arbiter, &run.efficiency.arbiter, 0),
            };

            let key = (
                role_name.to_string(),
                role_name,
                assignment.agent.clone(),
                assignment.model.clone(),
            );
            let bucket = buckets.entry(key).or_default();
            bucket.runs += 1;
            bucket.points += points;
            bucket.wall_ms_sum += metrics.wall_clock_ms as u128;
            bucket.tool_calls_sum += metrics.tool_call_count as u128;
            if let Some(total) = metrics.token_total() {
                bucket.token_sum += total as u128;
                bucket.token_count += 1;
            }
            if let Some(total) = metrics.byte_proxy_total() {
                bucket.byte_proxy_sum += total as u128;
                bucket.byte_proxy_count += 1;
            }
        }
    }

    let rows = buckets
        .into_iter()
        .map(|((_, role, agent, model), b)| {
            let runs = b.runs.max(1) as f64;
            AggregateRow {
                role,
                agent,
                model,
                runs: b.runs,
                points: b.points,
                avg_wall_seconds: (b.wall_ms_sum as f64 / runs) / 1_000.0,
                avg_tool_calls: b.tool_calls_sum as f64 / runs,
                avg_token_total: if b.token_count == 0 {
                    None
                } else {
                    Some(b.token_sum as f64 / b.token_count as f64)
                },
                avg_byte_proxy_total: if b.byte_proxy_count == 0 {
                    None
                } else {
                    Some(b.byte_proxy_sum as f64 / b.byte_proxy_count as f64)
                },
            }
        })
        .collect();

    Aggregates { rows }
}

fn seed_zero_family_rows(
    buckets: &mut BTreeMap<(String, &'static str, String, String), Bucket>,
    roles_to_emit: &[RoleAxis],
) {
    for role in roles_to_emit {
        let role_name = role_name(*role);
        for family in all_agent_families() {
            buckets
                .entry((
                    role_name.to_string(),
                    role_name,
                    family.to_string(),
                    default_orchestrator_model(family),
                ))
                .or_default();
        }
    }
}

fn role_name(role: RoleAxis) -> &'static str {
    match role {
        RoleAxis::PlannerA => "planner_a",
        RoleAxis::PlannerB => "planner_b",
        RoleAxis::Arbiter => "arbiter",
    }
}

fn default_orchestrator_model(family: &str) -> String {
    resolve_agent_model_pair(family)
        .map(|pair| pair.orchestrator)
        .unwrap_or_else(|| family.to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use chrono::Utc;
    use orbit_common::types::{
        EfficiencyMetrics, PlannerSlot, PlanningEfficiency, PlanningOutcome,
        PlanningRoleAssignment, PlanningRoles,
    };

    use super::*;

    #[test]
    fn append_run_keeps_all_concurrent_writes() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let scoreboard_dir = Arc::new(temp.path().to_path_buf());
        let writers = 32;
        let barrier = Arc::new(Barrier::new(writers));

        let handles: Vec<_> = (0..writers)
            .map(|index| {
                let scoreboard_dir = Arc::clone(&scoreboard_dir);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let run = test_run(format!("run-{index:02}"));
                    barrier.wait();
                    append_run(&scoreboard_dir, &run).expect("append run");
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("join writer thread");
        }

        let runs = load_runs(&scoreboard_dir).expect("load runs");
        assert_eq!(runs.len(), writers);

        let run_ids: BTreeSet<_> = runs.into_iter().map(|run| run.run_id).collect();
        let expected: BTreeSet<_> = (0..writers)
            .map(|index| format!("run-{index:02}"))
            .collect();
        assert_eq!(run_ids, expected);
    }

    #[test]
    fn aggregate_emits_zero_rows_for_known_families() {
        let aggregates = aggregate(&[], AggregateFilter::default());

        assert!(aggregates.rows.iter().any(|row| row.role == "planner_a"
            && row.agent == "grok"
            && row.model == "grok-4"
            && row.runs == 0));
        assert!(aggregates.rows.iter().any(|row| row.role == "planner_b"
            && row.agent == "grok"
            && row.model == "grok-4"
            && row.runs == 0));
        assert!(aggregates.rows.iter().any(|row| row.role == "arbiter"
            && row.agent == "grok"
            && row.model == "grok-4"
            && row.runs == 0));
    }

    fn test_run(run_id: String) -> PlanningDuelRun {
        PlanningDuelRun {
            run_id,
            task_id: "T-test".to_string(),
            completed_at: Utc::now(),
            roles: PlanningRoles {
                planner_a: role("codex", "gpt-5.5"),
                planner_b: role("claude", "opus"),
                arbiter: role("gemini", "pro"),
            },
            planner_a_artifact_path: "artifacts/planner-a.md".to_string(),
            planner_b_artifact_path: "artifacts/planner-b.md".to_string(),
            outcome: PlanningOutcome {
                winner: PlannerSlot::PlannerA,
                arbiter_rationale: "test winner".to_string(),
            },
            efficiency: PlanningEfficiency {
                planner_a: metrics(),
                planner_b: metrics(),
                arbiter: metrics(),
            },
        }
    }

    fn role(agent: &str, model: &str) -> PlanningRoleAssignment {
        PlanningRoleAssignment {
            agent: agent.to_string(),
            model: model.to_string(),
        }
    }

    fn metrics() -> EfficiencyMetrics {
        EfficiencyMetrics {
            wall_clock_ms: 1_000,
            tool_call_count: 1,
            token_usage: None,
            byte_proxy_total: None,
        }
    }
}
