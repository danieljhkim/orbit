//! Append-only log + aggregation for the planning-duel workflow.
//!
//! `.orbit/state/scoreboard/duel_plan.json` stores one row per planning duel. Each
//! row captures both planner proposals, the arbiter's winner, and per-role
//! efficiency metrics. Aggregates are computed on demand from the raw rows so
//! the report stays deterministic and schema-light.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use orbit_common::types::{OrbitError, PlannerSlot, PlanningDuelRun, all_agent_families};
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
    pub family: String,
    pub runs: u32,
    pub points: u32,
    pub avg_wall_seconds: f64,
    pub avg_tool_calls: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_token_total: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_byte_proxy_total: Option<f64>,
}

/// Aggregation result. Rows are sorted by role and family for
/// deterministic rendering.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Aggregates {
    pub rows: Vec<AggregateRow>,
}

/// One directed family-vs-family cell in the planning-duel matrix.
///
/// `wins` and `losses` are from the row family's perspective against the
/// column family. `runs` is the number of decided planner matchups observed
/// for that directed pair.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadToHeadCell {
    pub wins: u64,
    pub losses: u64,
    pub runs: u64,
}

/// Deterministic family-vs-family planning-duel matrix.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadToHeadMatrix {
    pub families: Vec<String>,
    pub cells: BTreeMap<String, BTreeMap<String, HeadToHeadCell>>,
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
    let mut buckets: BTreeMap<(String, &'static str, String), Bucket> = BTreeMap::new();

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
                assignment.family.to_string(),
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
        .map(|((_, role, family), b)| {
            let runs = b.runs.max(1) as f64;
            AggregateRow {
                role,
                family,
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

/// Aggregate planner outcomes into a directed family-vs-family matrix.
pub fn aggregate_head_to_head(runs: &[PlanningDuelRun]) -> HeadToHeadMatrix {
    let canonical_families: Vec<String> = all_agent_families()
        .iter()
        .map(|family| family.to_string())
        .collect();
    let mut cells = seed_head_to_head_cells(&canonical_families);

    for run in runs {
        let planner_a = run.roles.planner_a.family.to_string();
        let planner_b = run.roles.planner_b.family.to_string();
        ensure_head_to_head_family(&mut cells, &planner_a);
        ensure_head_to_head_family(&mut cells, &planner_b);

        match run.outcome.winner {
            PlannerSlot::PlannerA => {
                record_head_to_head(&mut cells, &planner_a, &planner_b, true);
                record_head_to_head(&mut cells, &planner_b, &planner_a, false);
            }
            PlannerSlot::PlannerB => {
                record_head_to_head(&mut cells, &planner_b, &planner_a, true);
                record_head_to_head(&mut cells, &planner_a, &planner_b, false);
            }
        }
    }

    let mut observed_families: Vec<String> = cells.keys().cloned().collect();
    observed_families.sort();
    let families: Vec<String> = canonical_families
        .into_iter()
        .chain(
            observed_families
                .into_iter()
                .filter(|family| !all_agent_families().contains(&family.as_str())),
        )
        .collect();

    HeadToHeadMatrix { families, cells }
}

fn seed_zero_family_rows(
    buckets: &mut BTreeMap<(String, &'static str, String), Bucket>,
    roles_to_emit: &[RoleAxis],
) {
    for role in roles_to_emit {
        let role_name = role_name(*role);
        for family in all_agent_families() {
            buckets
                .entry((role_name.to_string(), role_name, family.to_string()))
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

fn seed_head_to_head_cells(
    families: &[String],
) -> BTreeMap<String, BTreeMap<String, HeadToHeadCell>> {
    let mut cells = BTreeMap::new();
    for row_family in families {
        let row = cells
            .entry(row_family.clone())
            .or_insert_with(BTreeMap::new);
        for col_family in families {
            row.entry(col_family.clone()).or_default();
        }
    }
    cells
}

fn ensure_head_to_head_family(
    cells: &mut BTreeMap<String, BTreeMap<String, HeadToHeadCell>>,
    family: &str,
) {
    if cells.contains_key(family) {
        return;
    }

    let existing: Vec<String> = cells.keys().cloned().collect();
    let mut row = BTreeMap::new();
    for other in &existing {
        row.entry(other.clone()).or_default();
    }
    row.entry(family.to_string()).or_default();
    cells.insert(family.to_string(), row);

    for other in existing {
        cells
            .entry(other)
            .or_default()
            .entry(family.to_string())
            .or_default();
    }
}

fn record_head_to_head(
    cells: &mut BTreeMap<String, BTreeMap<String, HeadToHeadCell>>,
    row_family: &str,
    col_family: &str,
    won: bool,
) {
    let cell = cells
        .entry(row_family.to_string())
        .or_default()
        .entry(col_family.to_string())
        .or_default();
    cell.runs = cell.runs.saturating_add(1);
    if won {
        cell.wins = cell.wins.saturating_add(1);
    } else {
        cell.losses = cell.losses.saturating_add(1);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[cfg(test)]
mod tests;
