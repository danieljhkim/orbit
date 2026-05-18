use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::{
    Adr, AdrStatus, JobRun, JobRunState, Learning, OrbitError, PlannerSlot, Task, TaskStatus,
    all_agent_families, infer_agent_family_from_model, normalize_attribution_label,
    normalize_optional_attribution_label,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::planning_duel_scoreboard;
use crate::friction_store::StoredFrictionRecord;
use crate::{AuditToolCallCountsByRole, AuditToolCallCountsBySurfaceAndRole, AuditTopToolCall};
use orbit_common::utility::fs::atomic_write_text_volatile as write_atomic;

const SUMMARY_FILENAME: &str = "summary.json";
// v2 adds `task_review.threads`; v3 adds tasks_created/tasks_planned,
// per-(role, surface) tool call counts, top-level workflows_run, and a
// recent_7d window block. v4 adds per-agent knowledge counters and a
// planning-duel head-to-head matrix. v5 adds per-agent `friction.reported`
// (from append-only `.orbit/frictions/` records, matching `orbit.friction.stats`).
// Older readers ignore unknown fields.
const CURRENT_SCHEMA_VERSION: u32 = 5;
const TASK_REVIEW_THREADS_METRIC: &str = "task-review-threads";
const LEGACY_TASK_REVIEW_MESSAGES_METRIC: &str = "task-review-messages";
const RECENT_WINDOW_DAYS: i64 = 7;

type FamilyScoreboard = BTreeMap<String, BTreeMap<String, u64>>;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenSummary {
    pub total: u64,
    pub output: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DuelSummary {
    pub wins: u64,
    pub losses: u64,
    pub participated: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrSummary {
    pub review_comments: u64,
    pub merged_clean: u64,
    pub merged_with_revision: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskReviewSummary {
    #[serde(default, alias = "messages")]
    pub threads: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeSummary {
    pub learnings_created: u64,
    pub learning_votes_received: u64,
    pub adrs_created: u64,
    pub adrs_accepted: u64,
    pub adrs_proposed_open: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrictionSummary {
    /// Number of append-only friction records reported by this agent family.
    /// Sourced from `.orbit/frictions/` (via the same aggregation as
    /// `orbit.friction.stats` / `friction_stats`), not from legacy task status
    /// or `tool_calls_by_surface.friction`.
    pub reported: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSummary {
    pub tasks_completed: u64,
    #[serde(default)]
    pub tasks_created: u64,
    #[serde(default)]
    pub tasks_planned: u64,
    pub tokens: TokenSummary,
    pub duels: DuelSummary,
    pub pr: PrSummary,
    #[serde(default)]
    pub task_review: TaskReviewSummary,
    #[serde(default)]
    pub knowledge: KnowledgeSummary,
    #[serde(default)]
    pub friction: FrictionSummary,
    pub tool_calls: u64,
    #[serde(default)]
    pub failed_tool_calls: u64,
    /// Per-Orbit-surface tool call counts (e.g. `graph` → 56, `task` → 102).
    /// The surface key is the segment after the `orbit.` namespace prefix —
    /// see [`AuditToolCallCountsBySurfaceAndRole`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tool_calls_by_surface: BTreeMap<String, u64>,
}

/// Top-level "completed `orbit run` jobs" rollup. Not per-agent: a workflow
/// is a job-level concept and routinely fans out across multiple agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowRunCount {
    pub job_id: String,
    pub count: u64,
}

/// One row of the "most-called tools" leaderboard — `count` invocations of
/// `tool_name` attributed to `role`. Sourced from the audit log; restricted
/// to `orbit.*` tools by the SQL filter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopToolCall {
    pub role: String,
    pub tool_name: String,
    pub count: u64,
}

/// Headline totals over the most recent [`RECENT_WINDOW_DAYS`]. Carries no
/// per-agent breakdowns by design — the section is a "is this still being
/// used" recency signal, not a leaderboard.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentSummary {
    /// Lower bound of the window (inclusive), RFC3339.
    pub since: String,
    pub tasks_created: u64,
    pub tasks_completed: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tool_calls_by_surface: BTreeMap<String, u64>,
    pub workflows_run: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanningDuelSummary {
    pub head_to_head: planning_duel_scoreboard::HeadToHeadMatrix,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScoreboardSummary {
    pub schema_version: u32,
    pub generated_at: String,
    pub agents: BTreeMap<String, AgentSummary>,
    /// Top jobs by completed-run count, descending. Empty when the runtime
    /// passed no JobRun records (e.g. backward-compat callers).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workflows_run: Vec<WorkflowRunCount>,
    /// Top (role, tool_name) pairs across the audit log, restricted to
    /// `orbit.*` tool names. Already sorted desc by count.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_tools: Vec<TopToolCall>,
    /// Recency window for headline deltas on the public scoreboard.
    /// Optional so older readers / unit tests that don't wire it tolerate
    /// its absence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_7d: Option<RecentSummary>,
    /// Planning-duel reports that are not naturally per-agent columns.
    #[serde(default)]
    pub planning_duels: PlanningDuelSummary,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TokenScoreboardFile {
    #[serde(default)]
    agents: Vec<TokenAgentEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TokenAgentEntry {
    #[serde(rename = "agent")]
    _agent: String,
    /// Model key used for this token scoreboard row; per-invocation actual execution (from audit/token metrics, not run-level lineup).
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    total_tokens: u64,
    #[serde(default, alias = "output_tokens")]
    total_output_tokens: u64,
    #[serde(default)]
    total_tool_calls: u64,
}

/// Bundle of the optional inputs that have grown around the core task summary.
/// New callers should populate this struct;
/// the older `generate_summary*` thin wrappers stay for tests and any
/// caller that hasn't been updated yet.
#[derive(Debug, Clone)]
pub struct ScoreboardInputs<'a> {
    /// Per-(role) tool-call totals — drives the legacy `tool_calls`/
    /// `failed_tool_calls` columns.
    pub audit_tool_calls: &'a [AuditToolCallCountsByRole],
    /// Per-(role, surface) tool-call counts. All-time.
    pub audit_tool_calls_by_surface: &'a [AuditToolCallCountsBySurfaceAndRole],
    /// Per-(role, surface) tool-call counts windowed to the most recent
    /// [`RECENT_WINDOW_DAYS`]. Drives the `recent_7d.tool_calls_by_surface`
    /// totals.
    pub audit_tool_calls_by_surface_recent: &'a [AuditToolCallCountsBySurfaceAndRole],
    /// All persisted JobRun records — successful ones populate the
    /// `workflows_run` rollup; the lot drives the 7d workflows count.
    pub job_runs: &'a [JobRun],
    /// Top (role, tool_name) pairs across the audit log, sorted desc by
    /// count. Drives the "most-called tools" leaderboard.
    pub top_tool_calls: &'a [AuditTopToolCall],
    /// Workspace learning records, used for knowledge-stewardship counters.
    pub learnings: &'a [Learning],
    /// Per-learning vote counts keyed by learning ID.
    pub learning_vote_counts: &'a [(String, u64)],
    /// Workspace ADR records, used for knowledge-stewardship counters.
    pub adrs: &'a [Adr],
    /// Append-only friction records from `.orbit/frictions/`. Used to populate
    /// per-family `friction.reported` counts (so the dashboard `frict r` column
    /// and `orbit.friction.stats` agree, without using tool call surface counts).
    pub frictions: &'a [StoredFrictionRecord],
    /// Reference "now" for recency windowing. `None` means no recency
    /// section is emitted (used by legacy callers).
    pub now: Option<DateTime<Utc>>,
}

impl<'a> Default for ScoreboardInputs<'a> {
    fn default() -> Self {
        static EMPTY_AUDIT: [AuditToolCallCountsByRole; 0] = [];
        static EMPTY_SURFACE: [AuditToolCallCountsBySurfaceAndRole; 0] = [];
        static EMPTY_JOB: [JobRun; 0] = [];
        static EMPTY_TOP: [AuditTopToolCall; 0] = [];
        static EMPTY_LEARNING: [Learning; 0] = [];
        static EMPTY_VOTES: [(String, u64); 0] = [];
        static EMPTY_ADR: [Adr; 0] = [];
        Self {
            audit_tool_calls: &EMPTY_AUDIT,
            audit_tool_calls_by_surface: &EMPTY_SURFACE,
            audit_tool_calls_by_surface_recent: &EMPTY_SURFACE,
            job_runs: &EMPTY_JOB,
            top_tool_calls: &EMPTY_TOP,
            learnings: &EMPTY_LEARNING,
            learning_vote_counts: &EMPTY_VOTES,
            adrs: &EMPTY_ADR,
            frictions: &[],
            now: None,
        }
    }
}

pub fn generate_summary(
    scoreboard_dir: &Path,
    tasks: &[Task],
) -> Result<ScoreboardSummary, OrbitError> {
    generate_summary_with_inputs(scoreboard_dir, tasks, &ScoreboardInputs::default())
}

pub fn generate_summary_with_audit_tool_calls(
    scoreboard_dir: &Path,
    tasks: &[Task],
    audit_tool_calls: &[AuditToolCallCountsByRole],
) -> Result<ScoreboardSummary, OrbitError> {
    generate_summary_with_inputs(
        scoreboard_dir,
        tasks,
        &ScoreboardInputs {
            audit_tool_calls,
            ..ScoreboardInputs::default()
        },
    )
}

pub fn generate_summary_with_inputs(
    scoreboard_dir: &Path,
    tasks: &[Task],
    inputs: &ScoreboardInputs<'_>,
) -> Result<ScoreboardSummary, OrbitError> {
    let audit_tool_calls = inputs.audit_tool_calls;
    let mut agents: BTreeMap<String, AgentSummary> = BTreeMap::new();
    seed_known_family_agents(&mut agents);

    let pr = read_model_scoreboard(scoreboard_dir, "pr.json")?;
    overlay_nested_metric(&mut agents, &pr, "pr-review-comments", |summary, value| {
        summary.pr.review_comments = summary.pr.review_comments.saturating_add(value);
    });
    overlay_nested_metric(
        &mut agents,
        &pr,
        "pr-count-without-revision",
        |summary, value| {
            summary.pr.merged_clean = summary.pr.merged_clean.saturating_add(value);
        },
    );
    overlay_nested_metric(
        &mut agents,
        &pr,
        "pr-count-with-revision",
        |summary, value| {
            summary.pr.merged_with_revision = summary.pr.merged_with_revision.saturating_add(value);
        },
    );

    let task_review = read_model_scoreboard(scoreboard_dir, "task_review.json")?;
    overlay_nested_metric(
        &mut agents,
        &task_review,
        TASK_REVIEW_THREADS_METRIC,
        |summary, value| {
            summary.task_review.threads = summary.task_review.threads.saturating_add(value);
        },
    );

    for token_row in read_token_agents(scoreboard_dir)? {
        let Some(model) = token_row
            .model
            .as_deref()
            .map(family_key)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let summary = agents.entry(model).or_default();
        summary.tokens.total = summary.tokens.total.saturating_add(token_row.total_tokens);
        summary.tokens.output = summary
            .tokens
            .output
            .saturating_add(token_row.total_output_tokens);
        summary.tool_calls = summary
            .tool_calls
            .saturating_add(token_row.total_tool_calls);
    }

    overlay_audit_tool_calls(&mut agents, audit_tool_calls);
    overlay_audit_tool_calls_by_surface(&mut agents, inputs.audit_tool_calls_by_surface);

    // Planning-duel rows are the "who actually ran?" scoreboard projection:
    // metrics are recorded from invocation family + slot, while the stored
    // roles identify the selected family for each slot.
    let planning_duel_runs = planning_duel_scoreboard::load_runs(scoreboard_dir)?;
    for run in &planning_duel_runs {
        let planner_a = agents
            .entry(run.roles.planner_a.family.to_string())
            .or_default();
        planner_a.duels.participated = planner_a.duels.participated.saturating_add(1);
        let planner_b = agents
            .entry(run.roles.planner_b.family.to_string())
            .or_default();
        planner_b.duels.participated = planner_b.duels.participated.saturating_add(1);
        let arbiter = agents
            .entry(run.roles.arbiter.family.to_string())
            .or_default();
        arbiter.duels.participated = arbiter.duels.participated.saturating_add(1);

        match run.outcome.winner {
            PlannerSlot::PlannerA => {
                let planner_a = agents
                    .entry(run.roles.planner_a.family.to_string())
                    .or_default();
                planner_a.duels.wins = planner_a.duels.wins.saturating_add(1);
                let planner_b = agents
                    .entry(run.roles.planner_b.family.to_string())
                    .or_default();
                planner_b.duels.losses = planner_b.duels.losses.saturating_add(1);
            }
            PlannerSlot::PlannerB => {
                let planner_b = agents
                    .entry(run.roles.planner_b.family.to_string())
                    .or_default();
                planner_b.duels.wins = planner_b.duels.wins.saturating_add(1);
                let planner_a = agents
                    .entry(run.roles.planner_a.family.to_string())
                    .or_default();
                planner_a.duels.losses = planner_a.duels.losses.saturating_add(1);
            }
        }
    }

    overlay_knowledge_counters(&mut agents, inputs);
    overlay_friction_reported(&mut agents, inputs.frictions);

    for task in tasks {
        if matches!(task.status, TaskStatus::Done | TaskStatus::Archived)
            && let Some(model) = normalize_optional_attribution_label(
                task.implemented_by.as_deref(),
                task.implemented_by.as_deref(),
            )
        {
            let summary = agents.entry(family_key(&model)).or_default();
            summary.tasks_completed = summary.tasks_completed.saturating_add(1);
        }

        // Created/Planned count *all* statuses — see [T20260508-16]: rejected
        // and friction tasks still represent real work the agent produced.
        if let Some(label) = task
            .created_by
            .as_deref()
            .map(|raw| normalize_attribution_label(raw, None))
            .filter(|value| !value.is_empty())
        {
            let summary = agents.entry(family_key(&label)).or_default();
            summary.tasks_created = summary.tasks_created.saturating_add(1);
        }
        if let Some(label) = task
            .planned_by
            .as_deref()
            .map(|raw| normalize_attribution_label(raw, None))
            .filter(|value| !value.is_empty())
        {
            let summary = agents.entry(family_key(&label)).or_default();
            summary.tasks_planned = summary.tasks_planned.saturating_add(1);
        }
    }

    let workflows_run = aggregate_workflows_run(inputs.job_runs);
    let top_tools: Vec<TopToolCall> = inputs
        .top_tool_calls
        .iter()
        .map(|row| TopToolCall {
            role: row.role.clone(),
            tool_name: row.tool_name.clone(),
            count: row.total,
        })
        .collect();
    let recent_7d = inputs
        .now
        .map(|now| build_recent_summary(now, tasks, inputs));
    let planning_duels = PlanningDuelSummary {
        head_to_head: planning_duel_scoreboard::aggregate_head_to_head(&planning_duel_runs),
    };

    Ok(ScoreboardSummary {
        schema_version: CURRENT_SCHEMA_VERSION,
        generated_at: Utc::now().to_rfc3339(),
        agents,
        workflows_run,
        top_tools,
        recent_7d,
        planning_duels,
    })
}

fn aggregate_workflows_run(runs: &[JobRun]) -> Vec<WorkflowRunCount> {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for run in runs {
        if run.state == JobRunState::Success {
            *counts.entry(run.job_id.to_string()).or_insert(0) += 1;
        }
    }
    let mut rows: Vec<WorkflowRunCount> = counts
        .into_iter()
        .map(|(job_id, count)| WorkflowRunCount { job_id, count })
        .collect();
    // Highest run-count first; tie-break by job_id ASC for stable output.
    rows.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.job_id.cmp(&b.job_id)));
    rows
}

fn build_recent_summary(
    now: DateTime<Utc>,
    tasks: &[Task],
    inputs: &ScoreboardInputs<'_>,
) -> RecentSummary {
    let since = now - Duration::days(RECENT_WINDOW_DAYS);

    let mut tasks_created: u64 = 0;
    let mut tasks_completed: u64 = 0;
    for task in tasks {
        if task.created_at >= since {
            tasks_created = tasks_created.saturating_add(1);
        }
        if matches!(task.status, TaskStatus::Done | TaskStatus::Archived)
            && task_done_at(task).is_some_and(|done_at| done_at >= since)
        {
            tasks_completed = tasks_completed.saturating_add(1);
        }
    }

    let mut tool_calls_by_surface: BTreeMap<String, u64> = BTreeMap::new();
    for row in inputs.audit_tool_calls_by_surface_recent {
        *tool_calls_by_surface
            .entry(row.surface.clone())
            .or_insert(0) += row.total;
    }

    let workflows_run: u64 = inputs
        .job_runs
        .iter()
        .filter(|run| run.state == JobRunState::Success)
        .filter(|run| run_completed_at(run) >= since)
        .count() as u64;

    RecentSummary {
        since: since.to_rfc3339(),
        tasks_created,
        tasks_completed,
        tool_calls_by_surface,
        workflows_run,
    }
}

/// Best-effort timestamp for when a task entered `done`/`archived`.
/// Task history is no longer embedded in the public task DTO, so summary
/// generation uses the envelope `updated_at` timestamp.
fn task_done_at(task: &Task) -> Option<DateTime<Utc>> {
    Some(task.updated_at)
}

/// Best-effort completion timestamp for a JobRun. `finished_at` is set when
/// the run terminates; the fallback to `created_at` keeps the recency
/// filter conservative for legacy rows that pre-date that field.
fn run_completed_at(run: &JobRun) -> DateTime<Utc> {
    run.finished_at.unwrap_or(run.created_at)
}

pub fn write_summary(
    scoreboard_dir: &Path,
    summary: &ScoreboardSummary,
) -> Result<std::path::PathBuf, OrbitError> {
    let path = scoreboard_dir.join(SUMMARY_FILENAME);
    let raw = serde_json::to_string_pretty(summary)
        .map_err(|e| OrbitError::Io(format!("serialize summary.json: {e}")))?;
    write_atomic(&path, &format!("{raw}\n"))?;
    Ok(path)
}

pub fn summary_path(scoreboard_dir: &Path) -> std::path::PathBuf {
    scoreboard_dir.join(SUMMARY_FILENAME)
}

fn read_model_scoreboard(
    scoreboard_dir: &Path,
    file_name: &str,
) -> Result<FamilyScoreboard, OrbitError> {
    let path = scoreboard_dir.join(file_name);
    if !path.exists() {
        return Ok(FamilyScoreboard::new());
    }
    let raw =
        fs::read_to_string(&path).map_err(|e| OrbitError::Io(format!("read {file_name}: {e}")))?;
    if raw.trim().is_empty() {
        return Ok(FamilyScoreboard::new());
    }
    let parsed: Value = serde_json::from_str(&raw)
        .map_err(|e| OrbitError::Io(format!("parse {file_name}: {e}")))?;
    normalize_model_scoreboard(parsed)
}

fn read_token_agents(scoreboard_dir: &Path) -> Result<Vec<TokenAgentEntry>, OrbitError> {
    let path = scoreboard_dir.join("tokens.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw =
        fs::read_to_string(&path).map_err(|e| OrbitError::Io(format!("read tokens.json: {e}")))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed: TokenScoreboardFile = serde_json::from_str(&raw)
        .map_err(|e| OrbitError::Io(format!("parse tokens.json: {e}")))?;
    Ok(parsed.agents)
}

fn overlay_nested_metric(
    agents: &mut BTreeMap<String, AgentSummary>,
    scoreboard: &FamilyScoreboard,
    metric: &str,
    mut apply: impl FnMut(&mut AgentSummary, u64),
) {
    let Some(by_family) = scoreboard.get(metric) else {
        return;
    };

    for (family, value) in by_family {
        let summary = agents.entry(family_key(family)).or_default();
        apply(summary, *value);
    }
}

fn overlay_audit_tool_calls_by_surface(
    agents: &mut BTreeMap<String, AgentSummary>,
    rows: &[AuditToolCallCountsBySurfaceAndRole],
) {
    for row in rows {
        let family = family_key(&row.role);
        if family.is_empty() {
            continue;
        }
        let summary = agents.entry(family).or_default();
        let entry = summary
            .tool_calls_by_surface
            .entry(row.surface.clone())
            .or_insert(0);
        *entry = entry.saturating_add(row.total);
    }
}

fn overlay_audit_tool_calls(
    agents: &mut BTreeMap<String, AgentSummary>,
    audit_tool_calls: &[AuditToolCallCountsByRole],
) {
    let mut by_family: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for row in audit_tool_calls {
        let family = family_key(&row.role);
        if family.is_empty() {
            continue;
        }
        let entry = by_family.entry(family).or_default();
        entry.0 = entry.0.saturating_add(row.total);
        entry.1 = entry.1.saturating_add(row.failed);
    }

    for (family, (total, failed)) in by_family {
        let summary = agents.entry(family).or_default();
        // Total competes with token scoreboard data; failures only exist in audit rows.
        summary.tool_calls = summary.tool_calls.max(total);
        summary.failed_tool_calls = summary.failed_tool_calls.saturating_add(failed);
    }
}

fn overlay_knowledge_counters(
    agents: &mut BTreeMap<String, AgentSummary>,
    inputs: &ScoreboardInputs<'_>,
) {
    for learning in inputs.learnings {
        let Some(created_by) = learning
            .created_by
            .as_deref()
            .map(|raw| normalize_attribution_label(raw, None))
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let summary = agents.entry(family_key(&created_by)).or_default();
        summary.knowledge.learnings_created = summary.knowledge.learnings_created.saturating_add(1);
        summary.knowledge.learning_votes_received = summary
            .knowledge
            .learning_votes_received
            .saturating_add(learning_vote_count(
                inputs.learning_vote_counts,
                &learning.id,
            ));
    }

    for adr in inputs.adrs {
        let owner = normalize_attribution_label(&adr.owner, None);
        if owner.is_empty() {
            continue;
        }
        let summary = agents.entry(family_key(&owner)).or_default();
        summary.knowledge.adrs_created = summary.knowledge.adrs_created.saturating_add(1);
        if adr.status == AdrStatus::Accepted || adr.accepted_at.is_some() {
            summary.knowledge.adrs_accepted = summary.knowledge.adrs_accepted.saturating_add(1);
        }
        if adr.status == AdrStatus::Proposed {
            summary.knowledge.adrs_proposed_open =
                summary.knowledge.adrs_proposed_open.saturating_add(1);
        }
    }
}

fn overlay_friction_reported(
    agents: &mut BTreeMap<String, AgentSummary>,
    frictions: &[StoredFrictionRecord],
) {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for stored in frictions {
        let family = {
            let normalized = normalize_optional_attribution_label(Some(&stored.record.model), None)
                .unwrap_or_default();
            infer_agent_family_from_model(&normalized).unwrap_or(normalized)
        };
        *counts.entry(family).or_insert(0) += 1;
    }
    for (family, count) in counts {
        let summary = agents.entry(family).or_default();
        summary.friction.reported = count;
    }
}

fn learning_vote_count(counts: &[(String, u64)], id: &str) -> u64 {
    counts
        .iter()
        .find_map(|(learning_id, count)| (learning_id == id).then_some(*count))
        .unwrap_or(0)
}

fn family_key(label: &str) -> String {
    let normalized = normalize_attribution_label(label, None);
    infer_agent_family_from_model(&normalized).unwrap_or(normalized)
}

fn seed_known_family_agents(agents: &mut BTreeMap<String, AgentSummary>) {
    for family in all_agent_families() {
        agents.entry(family.to_string()).or_default();
    }
}

fn normalize_model_scoreboard(parsed: Value) -> Result<FamilyScoreboard, OrbitError> {
    let mut normalized = FamilyScoreboard::new();
    let Value::Object(metrics) = parsed else {
        return Err(OrbitError::Io(
            "scoreboard json must be an object".to_string(),
        ));
    };

    for (metric, metric_value) in metrics {
        let Value::Object(entries) = metric_value else {
            continue;
        };
        let family_entries = normalized
            .entry(canonical_scoreboard_metric(&metric).to_string())
            .or_default();
        for (first_key, first_value) in entries {
            match first_value {
                Value::Number(number) => {
                    let value = number.as_u64().ok_or_else(|| {
                        OrbitError::Io("scoreboard counter must be u64".to_string())
                    })?;
                    *family_entries.entry(family_key(&first_key)).or_insert(0) += value;
                }
                Value::Object(inner) => {
                    for (family, value) in inner {
                        let Value::Number(number) = value else {
                            continue;
                        };
                        let count = number.as_u64().ok_or_else(|| {
                            OrbitError::Io("scoreboard counter must be u64".to_string())
                        })?;
                        *family_entries.entry(family_key(&family)).or_insert(0) += count;
                    }
                }
                _ => {}
            }
        }
    }

    Ok(normalized)
}

fn canonical_scoreboard_metric(metric: &str) -> &str {
    match metric {
        LEGACY_TASK_REVIEW_MESSAGES_METRIC => TASK_REVIEW_THREADS_METRIC,
        _ => metric,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_overlays_audit_tool_call_counts_by_normalized_model() {
        let temp = tempfile::tempdir().expect("create tempdir");

        let summary = generate_summary_with_audit_tool_calls(
            temp.path(),
            &[],
            &[
                AuditToolCallCountsByRole {
                    role: "codex / gpt-5".to_string(),
                    total: 2,
                    failed: 1,
                },
                AuditToolCallCountsByRole {
                    role: "gpt-5".to_string(),
                    total: 1,
                    failed: 1,
                },
            ],
        )
        .expect("generate summary");

        let codex = summary.agents.get("codex").expect("codex summary");
        assert_eq!(codex.tool_calls, 3);
        assert_eq!(codex.failed_tool_calls, 2);
    }

    #[test]
    fn summary_includes_zero_rows_for_known_families() {
        let temp = tempfile::tempdir().expect("create tempdir");

        let summary = generate_summary(temp.path(), &[]).expect("generate summary");

        let grok = summary.agents.get("grok").expect("grok summary");
        assert_eq!(grok.tasks_completed, 0);
        assert_eq!(grok.duels.participated, 0);
        assert_eq!(grok.task_review.threads, 0);
    }

    #[test]
    fn summary_agent_keys_are_families_not_models() {
        let temp = tempfile::tempdir().expect("create tempdir");
        fs::create_dir_all(temp.path()).expect("create scoreboard dir");
        fs::write(
            temp.path().join("tokens.json"),
            r#"{
              "agents": [
                { "agent": "codex", "model": "gpt-5.5", "total_tokens": 1 },
                { "agent": "claude", "model": "claude-opus-4-7", "total_tokens": 1 },
                { "agent": "grok", "model": "grok-4", "total_tokens": 1 }
              ]
            }"#,
        )
        .expect("write tokens scoreboard");

        let summary = generate_summary(temp.path(), &[]).expect("generate summary");
        for forbidden in ["grok-4", "claude-opus-4-7", "gpt-5.5"] {
            assert!(
                !summary.agents.contains_key(forbidden),
                "model key leaked into summary agents: {forbidden}"
            );
        }
        for family in ["codex", "claude", "gemini", "grok"] {
            assert!(summary.agents.contains_key(family));
        }
    }

    #[test]
    fn audit_tool_calls_do_not_double_count_token_scoreboard_tool_calls() {
        let temp = tempfile::tempdir().expect("create tempdir");
        fs::create_dir_all(temp.path()).expect("create scoreboard dir");
        fs::write(
            temp.path().join("tokens.json"),
            r#"{
              "agents": [
                {
                  "agent": "codex",
                  "model": "gpt-5",
                  "total_tokens": 10,
                  "total_output_tokens": 4,
                  "total_tool_calls": 5
                }
              ]
            }"#,
        )
        .expect("write tokens scoreboard");

        let summary = generate_summary_with_audit_tool_calls(
            temp.path(),
            &[],
            &[AuditToolCallCountsByRole {
                role: "gpt-5".to_string(),
                total: 3,
                failed: 2,
            }],
        )
        .expect("generate summary");

        let codex = summary.agents.get("codex").expect("codex summary");
        assert_eq!(codex.tokens.total, 10);
        assert_eq!(codex.tokens.output, 4);
        assert_eq!(codex.tool_calls, 5);
        assert_eq!(codex.failed_tool_calls, 2);
    }

    #[test]
    fn audit_tool_calls_win_when_larger_than_token_scoreboard_tool_calls() {
        let temp = tempfile::tempdir().expect("create tempdir");
        fs::create_dir_all(temp.path()).expect("create scoreboard dir");
        fs::write(
            temp.path().join("tokens.json"),
            r#"{
              "agents": [
                {
                  "agent": "codex",
                  "model": "gpt-5",
                  "total_tokens": 10,
                  "total_output_tokens": 4,
                  "total_tool_calls": 2
                }
              ]
            }"#,
        )
        .expect("write tokens scoreboard");

        let summary = generate_summary_with_audit_tool_calls(
            temp.path(),
            &[],
            &[AuditToolCallCountsByRole {
                role: "gpt-5".to_string(),
                total: 7,
                failed: 3,
            }],
        )
        .expect("generate summary");

        let codex = summary.agents.get("codex").expect("codex summary");
        assert_eq!(codex.tokens.total, 10);
        assert_eq!(codex.tokens.output, 4);
        assert_eq!(codex.tool_calls, 7);
        assert_eq!(codex.failed_tool_calls, 3);
    }

    #[test]
    fn summary_exposes_task_review_threads_separately_from_pr_comments() {
        let temp = tempfile::tempdir().expect("create tempdir");
        fs::create_dir_all(temp.path()).expect("create scoreboard dir");
        fs::write(
            temp.path().join("task_review.json"),
            r#"{"task-review-threads":{"gpt-reviewer":2}}"#,
        )
        .expect("write task review scoreboard");
        fs::write(
            temp.path().join("pr.json"),
            r#"{"pr-review-comments":{"gpt-reviewer":1}}"#,
        )
        .expect("write pr scoreboard");

        let summary = generate_summary(temp.path(), &[]).expect("generate summary");

        assert_eq!(summary.schema_version, CURRENT_SCHEMA_VERSION);
        let reviewer = summary.agents.get("codex").expect("reviewer summary");
        assert_eq!(reviewer.task_review.threads, 2);
        assert_eq!(reviewer.pr.review_comments, 1);
    }

    #[test]
    fn summary_counts_tasks_created_and_planned_across_all_statuses() {
        let temp = tempfile::tempdir().expect("create tempdir");

        // Mix of statuses including ones excluded from `tasks_completed`.
        let tasks = vec![
            test_task("T1", TaskStatus::Done, "claude-opus-4-7", "claude-opus-4-7"),
            test_task("T2", TaskStatus::Backlog, "claude-opus-4-7", "gpt-5.5"),
            test_task(
                "T3",
                TaskStatus::Rejected,
                "claude-opus-4-7",
                "claude-opus-4-7",
            ),
            test_task("T4", TaskStatus::Friction, "gpt-5.5", "gpt-5.5"),
            test_task_no_attrib("T5", TaskStatus::Done),
        ];

        let summary = generate_summary(temp.path(), &tasks).expect("generate summary");

        let claude = summary.agents.get("claude").expect("claude summary");
        // Three tasks were created by claude (Done, Backlog, Rejected).
        assert_eq!(claude.tasks_created, 3);
        // Two were planned by claude (Done, Rejected).
        assert_eq!(claude.tasks_planned, 2);
        // Only Done counts toward Completed (no `task.model` here, so it
        // attributes via `implemented_by`-equivalent — but we left model None;
        // verify the attribution still ignores Backlog/Rejected/Friction).
        // T1 (Done) has implemented_by=None and model=None, so it does not
        // attribute to Completed.
        assert_eq!(claude.tasks_completed, 0);

        let codex = summary.agents.get("codex").expect("codex summary");
        assert_eq!(codex.tasks_created, 1); // T4
        assert_eq!(codex.tasks_planned, 2); // T2, T4

        // T5 has no created_by/planned_by — must not crash and must not
        // create a phantom agent bucket.
        assert!(!summary.agents.contains_key(""));
    }

    #[test]
    fn summary_counts_knowledge_artifacts_by_author_family() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let learnings = vec![
            test_learning("L20260518-1", Some("gpt-5.5")),
            test_learning("L20260518-2", Some("claude-opus-4-7")),
            test_learning("L20260518-3", None),
        ];
        let learning_votes = vec![
            ("L20260518-1".to_string(), 2),
            ("L20260518-2".to_string(), 1),
        ];
        let now = Utc::now();
        let adrs = vec![
            test_adr("ADR-0001", "codex", AdrStatus::Accepted, Some(now)),
            test_adr("ADR-0002", "gpt-5.5", AdrStatus::Proposed, None),
            test_adr(
                "ADR-0003",
                "claude-opus-4-7",
                AdrStatus::Superseded,
                Some(now),
            ),
        ];

        let summary = generate_summary_with_inputs(
            temp.path(),
            &[],
            &ScoreboardInputs {
                learnings: &learnings,
                learning_vote_counts: &learning_votes,
                adrs: &adrs,
                ..ScoreboardInputs::default()
            },
        )
        .expect("generate summary");

        let codex = summary.agents.get("codex").expect("codex summary");
        assert_eq!(codex.knowledge.learnings_created, 1);
        assert_eq!(codex.knowledge.learning_votes_received, 2);
        assert_eq!(codex.knowledge.adrs_created, 2);
        assert_eq!(codex.knowledge.adrs_accepted, 1);
        assert_eq!(codex.knowledge.adrs_proposed_open, 1);

        let claude = summary.agents.get("claude").expect("claude summary");
        assert_eq!(claude.knowledge.learnings_created, 1);
        assert_eq!(claude.knowledge.learning_votes_received, 1);
        assert_eq!(claude.knowledge.adrs_created, 1);
        assert_eq!(claude.knowledge.adrs_accepted, 1);
        assert_eq!(claude.knowledge.adrs_proposed_open, 0);
    }

    #[test]
    fn summary_overlays_per_surface_tool_call_counts() {
        let temp = tempfile::tempdir().expect("create tempdir");

        let surface_rows = vec![
            AuditToolCallCountsBySurfaceAndRole {
                surface: "graph".to_string(),
                role: "claude-opus-4-7".to_string(),
                total: 56,
                failed: 2,
            },
            AuditToolCallCountsBySurfaceAndRole {
                surface: "graph".to_string(),
                role: "gpt-5.5".to_string(),
                total: 697,
                failed: 5,
            },
            AuditToolCallCountsBySurfaceAndRole {
                surface: "task".to_string(),
                role: "gpt-5.5".to_string(),
                total: 410,
                failed: 1,
            },
        ];

        let summary = generate_summary_with_inputs(
            temp.path(),
            &[],
            &ScoreboardInputs {
                audit_tool_calls_by_surface: &surface_rows,
                ..ScoreboardInputs::default()
            },
        )
        .expect("generate summary");

        let claude = summary.agents.get("claude").expect("claude summary");
        assert_eq!(claude.tool_calls_by_surface.get("graph").copied(), Some(56));
        assert_eq!(claude.tool_calls_by_surface.get("task"), None);

        let codex = summary.agents.get("codex").expect("codex summary");
        assert_eq!(codex.tool_calls_by_surface.get("graph").copied(), Some(697));
        assert_eq!(codex.tool_calls_by_surface.get("task").copied(), Some(410));
    }

    #[test]
    fn summary_aggregates_workflows_run_for_successful_runs() {
        let temp = tempfile::tempdir().expect("create tempdir");

        let now = Utc::now();
        let runs = vec![
            test_job_run("r1", "task_local_pipeline", JobRunState::Success, now),
            test_job_run("r2", "task_local_pipeline", JobRunState::Success, now),
            test_job_run("r3", "task_local_pipeline", JobRunState::Failed, now),
            test_job_run("r4", "task_auto_pipeline", JobRunState::Success, now),
            test_job_run("r5", "task_pr_pipeline", JobRunState::Cancelled, now),
        ];

        let summary = generate_summary_with_inputs(
            temp.path(),
            &[],
            &ScoreboardInputs {
                job_runs: &runs,
                ..ScoreboardInputs::default()
            },
        )
        .expect("generate summary");

        // Sorted descending by count, then job_id ascending.
        assert_eq!(
            summary.workflows_run,
            vec![
                WorkflowRunCount {
                    job_id: "task_local_pipeline".to_string(),
                    count: 2,
                },
                WorkflowRunCount {
                    job_id: "task_auto_pipeline".to_string(),
                    count: 1,
                },
            ]
        );
    }

    #[test]
    fn recent_7d_filters_tasks_workflows_and_surface_calls_by_window() {
        let temp = tempfile::tempdir().expect("create tempdir");

        let now = Utc::now();
        let inside = now - chrono::Duration::days(3);
        let outside = now - chrono::Duration::days(30);

        // Two created in-window, one outside.
        let mut t_inside = test_task(
            "T-in",
            TaskStatus::Done,
            "claude-opus-4-7",
            "claude-opus-4-7",
        );
        t_inside.created_at = inside;
        t_inside.updated_at = inside;

        let mut t_inside2 = test_task("T-in2", TaskStatus::Backlog, "gpt-5.5", "gpt-5.5");
        t_inside2.created_at = inside;

        let mut t_outside = test_task(
            "T-out",
            TaskStatus::Done,
            "claude-opus-4-7",
            "claude-opus-4-7",
        );
        t_outside.created_at = outside;
        t_outside.updated_at = outside; // legacy: no history transition
        // No history on t_outside — task_done_at falls back to updated_at.

        let tasks = vec![t_inside, t_inside2, t_outside];

        let surface_recent = vec![AuditToolCallCountsBySurfaceAndRole {
            surface: "graph".to_string(),
            role: "claude-opus-4-7".to_string(),
            total: 12,
            failed: 0,
        }];

        let runs = vec![
            test_job_run(
                "r-recent",
                "task_local_pipeline",
                JobRunState::Success,
                inside,
            ),
            test_job_run(
                "r-old",
                "task_local_pipeline",
                JobRunState::Success,
                outside,
            ),
        ];

        let summary = generate_summary_with_inputs(
            temp.path(),
            &tasks,
            &ScoreboardInputs {
                audit_tool_calls_by_surface_recent: &surface_recent,
                job_runs: &runs,
                now: Some(now),
                ..ScoreboardInputs::default()
            },
        )
        .expect("generate summary");

        let recent = summary
            .recent_7d
            .expect("recent_7d populated when now is set");
        // Two tasks created in window (T-in, T-in2). T-out is older.
        assert_eq!(recent.tasks_created, 2);
        // One task transitioned to Done in window (T-in). T-out's
        // updated_at is older than the window.
        assert_eq!(recent.tasks_completed, 1);
        // Surface row total flows through.
        assert_eq!(recent.tool_calls_by_surface.get("graph").copied(), Some(12));
        // Only the recent run counts.
        assert_eq!(recent.workflows_run, 1);
    }

    #[test]
    fn summary_passes_top_tools_through_unchanged() {
        let temp = tempfile::tempdir().expect("create tempdir");

        let rows = vec![
            AuditTopToolCall {
                role: "gpt-5.5".to_string(),
                tool_name: "orbit.graph.show".to_string(),
                total: 355,
            },
            AuditTopToolCall {
                role: "claude-opus-4-7".to_string(),
                tool_name: "orbit.graph.search".to_string(),
                total: 45,
            },
        ];

        let summary = generate_summary_with_inputs(
            temp.path(),
            &[],
            &ScoreboardInputs {
                top_tool_calls: &rows,
                ..ScoreboardInputs::default()
            },
        )
        .expect("generate summary");

        assert_eq!(
            summary.top_tools,
            vec![
                TopToolCall {
                    role: "gpt-5.5".to_string(),
                    tool_name: "orbit.graph.show".to_string(),
                    count: 355,
                },
                TopToolCall {
                    role: "claude-opus-4-7".to_string(),
                    tool_name: "orbit.graph.search".to_string(),
                    count: 45,
                },
            ]
        );
    }

    #[test]
    fn recent_7d_absent_when_now_not_provided() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let summary = generate_summary(temp.path(), &[]).expect("generate summary");
        assert!(summary.recent_7d.is_none());
    }

    fn test_task(
        id: &str,
        status: TaskStatus,
        created_by: &str,
        planned_by: &str,
    ) -> orbit_common::types::Task {
        let mut task = test_task_no_attrib(id, status);
        task.created_by = Some(created_by.to_string());
        task.planned_by = Some(planned_by.to_string());
        task
    }

    fn test_task_no_attrib(id: &str, status: TaskStatus) -> orbit_common::types::Task {
        use orbit_common::types::{Task, TaskPriority, TaskType};
        Task {
            id: id.to_string(),
            title: id.to_string(),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            tags: Vec::new(),
            plan: String::new(),
            execution_summary: String::new(),
            context_files: Vec::new(),
            created_by: None,
            planned_by: None,
            implemented_by: None,
            status,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Chore,
            pr_status: None,
            external_refs: Vec::new(),
            relations: Vec::new(),
            job_run_id: None,
            crew: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn test_learning(id: &str, created_by: Option<&str>) -> Learning {
        use orbit_common::types::{LearningScope, LearningStatus};
        Learning {
            id: id.to_string(),
            status: LearningStatus::Active,
            scope: LearningScope::default(),
            summary: id.to_string(),
            body: String::new(),
            evidence: Vec::new(),
            supersedes: None,
            superseded_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: created_by.map(str::to_string),
            priority: None,
        }
    }

    fn test_adr(
        id: &str,
        owner: &str,
        status: AdrStatus,
        accepted_at: Option<DateTime<Utc>>,
    ) -> Adr {
        Adr {
            id: id.to_string(),
            title: id.to_string(),
            status,
            owner: owner.to_string(),
            created_at: Utc::now(),
            accepted_at,
            last_updated: Utc::now(),
            related_features: Vec::new(),
            related_tasks: Vec::new(),
            supersedes: Vec::new(),
            superseded_by: None,
            legacy_ids: Vec::new(),
            validation_warnings: Vec::new(),
            legacy_validation: Default::default(),
        }
    }

    fn test_job_run(
        run_id: &str,
        job_id: &str,
        state: JobRunState,
        finished_at: chrono::DateTime<Utc>,
    ) -> JobRun {
        JobRun {
            run_id: run_id.to_string(),
            job_id: job_id.to_string(),
            attempt: 1,
            state,
            scheduled_at: finished_at,
            started_at: Some(finished_at),
            finished_at: Some(finished_at),
            duration_ms: Some(0),
            created_at: finished_at,
            pid: None,
            pid_start_time: None,
            input: None,
            retry_source_run_id: None,
            knowledge_metrics: None,
            resolved_crew: None,
            planner_model: None,
            implementer_model: None,
            reviewer_model: None,
            steps: Vec::new(),
        }
    }

    #[test]
    fn summary_reads_legacy_task_review_messages_as_threads() {
        let temp = tempfile::tempdir().expect("create tempdir");
        fs::create_dir_all(temp.path()).expect("create scoreboard dir");
        fs::write(
            temp.path().join("task_review.json"),
            r#"{"task-review-messages":{"gpt-reviewer":2}}"#,
        )
        .expect("write legacy task review scoreboard");

        let summary = generate_summary(temp.path(), &[]).expect("generate summary");

        let reviewer = summary.agents.get("codex").expect("reviewer summary");
        assert_eq!(reviewer.task_review.threads, 2);
    }

    #[test]
    fn summary_exposes_friction_reported_counts_from_records() {
        // Deterministic test per ORB-00143: seeds friction records for >=2 families
        // and asserts the generated scoreboard exposes nonzero `friction.reported`
        // (and zero for families with none). Uses the inputs path so it does not
        // depend on disk state or legacy task.status=friction.
        let temp = tempfile::tempdir().expect("create tempdir");

        let frictions: Vec<crate::friction_store::StoredFrictionRecord> = vec![
            crate::friction_store::StoredFrictionRecord {
                record: orbit_common::types::FrictionRecord {
                    id: "F001".to_string(),
                    model: "codex".to_string(),
                    created_at: Utc::now(),
                    status: orbit_common::types::FrictionStatus::Open,
                    tags: vec![],
                    resolved_at: None,
                    during_task: None,
                    resolved_by_task: None,
                    body: "seed for codex family".to_string(),
                },
                path: std::path::PathBuf::from("frictions/2026-05/F001.md"),
            },
            crate::friction_store::StoredFrictionRecord {
                record: orbit_common::types::FrictionRecord {
                    id: "F002".to_string(),
                    model: "claude-3-opus".to_string(),
                    created_at: Utc::now(),
                    status: orbit_common::types::FrictionStatus::Resolved,
                    tags: vec!["test".to_string()],
                    resolved_at: Some(Utc::now()),
                    during_task: None,
                    resolved_by_task: None,
                    body: "seed for claude family (normalized)".to_string(),
                },
                path: std::path::PathBuf::from("frictions/2026-05/F002.md"),
            },
        ];

        let summary = generate_summary_with_inputs(
            temp.path(),
            &[],
            &ScoreboardInputs {
                frictions: &frictions,
                ..ScoreboardInputs::default()
            },
        )
        .expect("generate summary with seeded frictions");

        let codex = summary.agents.get("codex").expect("codex summary");
        assert_eq!(
            codex.friction.reported, 1,
            "codex should report 1 friction record"
        );

        let claude = summary.agents.get("claude").expect("claude summary");
        assert_eq!(
            claude.friction.reported, 1,
            "claude (from claude-3-opus) should report 1"
        );

        let gemini = summary.agents.get("gemini").expect("gemini summary");
        assert_eq!(
            gemini.friction.reported, 0,
            "gemini with no records must expose 0, not fall back"
        );

        let grok = summary.agents.get("grok").expect("grok summary");
        assert_eq!(grok.friction.reported, 0);
    }
}
