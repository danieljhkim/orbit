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
// v6 ([ORB-00337]) adds top-level `window` + `window_since` fields and the
// `ScoreboardInputs.window` plumbing — snapshot-sourced per-agent fields
// (`tokens`, `pr`, `duels`, `task_review.threads`) zero out under non-`All`
// windows because we lack a timestamped snapshot log to filter against.
// Older readers ignore unknown fields.
const CURRENT_SCHEMA_VERSION: u32 = 6;
const TASK_REVIEW_THREADS_METRIC: &str = "task-review-threads";
const LEGACY_TASK_REVIEW_MESSAGES_METRIC: &str = "task-review-messages";
const RECENT_WINDOW_DAYS: i64 = 7;

type FamilyScoreboard = BTreeMap<String, BTreeMap<String, u64>>;

/// Time window for a scoreboard summary. `All` is the legacy lifetime view —
/// every non-`All` variant carries a finite `duration()` used as the cutoff
/// for windowed source filtering inside [`generate_summary_with_inputs`].
///
/// String forms (used in the dashboard query param and the serialized
/// `ScoreboardSummary.window` field): `1h`, `24h`, `7d`, `30d`, `all`.
///
/// Snapshot-sourced fields (`tokens`, `pr`, `duels`, `task_review.threads`)
/// have no per-event timestamp, so they zero out under any non-`All` window;
/// see the v6 schema comment. Per-(role) audit aggregates are filtered at
/// query time by the caller (the runtime in `orbit-core`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScoreboardWindow {
    Hour,
    Day,
    Week,
    Month,
    #[default]
    All,
}

impl ScoreboardWindow {
    /// Window length, or `None` for `All` (no cutoff).
    pub fn duration(self) -> Option<Duration> {
        match self {
            ScoreboardWindow::Hour => Some(Duration::hours(1)),
            ScoreboardWindow::Day => Some(Duration::hours(24)),
            ScoreboardWindow::Week => Some(Duration::days(7)),
            ScoreboardWindow::Month => Some(Duration::days(30)),
            ScoreboardWindow::All => None,
        }
    }

    /// Canonical short string used in the dashboard query param and the
    /// serialized `ScoreboardSummary.window` field.
    pub fn as_str(self) -> &'static str {
        match self {
            ScoreboardWindow::Hour => "1h",
            ScoreboardWindow::Day => "24h",
            ScoreboardWindow::Week => "7d",
            ScoreboardWindow::Month => "30d",
            ScoreboardWindow::All => "all",
        }
    }

    /// Parse a canonical window string. Accepts exactly `"1h"`, `"24h"`,
    /// `"7d"`, `"30d"`, or `"all"`. Any other input returns
    /// [`OrbitError::InvalidInput`] so HTTP callers can render an exact 400.
    pub fn from_str(value: &str) -> Result<Self, OrbitError> {
        match value {
            "1h" => Ok(ScoreboardWindow::Hour),
            "24h" => Ok(ScoreboardWindow::Day),
            "7d" => Ok(ScoreboardWindow::Week),
            "30d" => Ok(ScoreboardWindow::Month),
            "all" => Ok(ScoreboardWindow::All),
            other => Err(OrbitError::InvalidInput(format!(
                "unknown scoreboard window '{other}' (expected one of 1h, 24h, 7d, 30d, all)"
            ))),
        }
    }
}

impl TryFrom<&str> for ScoreboardWindow {
    type Error = OrbitError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        ScoreboardWindow::from_str(value)
    }
}

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
    /// Recency window for headline deltas on the public scoreboard. The
    /// 7d boundary here is independent of the user-selected scoreboard
    /// `window` — `recent_7d` is a "is this still being used" signal,
    /// always over the same fixed period, not a leaderboard.
    /// Optional so older readers / unit tests that don't wire it tolerate
    /// its absence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_7d: Option<RecentSummary>,
    /// Planning-duel reports that are not naturally per-agent columns.
    #[serde(default)]
    pub planning_duels: PlanningDuelSummary,
    /// Selected scoreboard window in canonical short form (`"1h"`,
    /// `"24h"`, `"7d"`, `"30d"`, `"all"`). v6+. Older readers tolerate
    /// the field's absence via `#[serde(default)]`.
    #[serde(default)]
    pub window: String,
    /// RFC3339 lower bound of the scoreboard window, or `None` when the
    /// window is `All` (lifetime). v6+.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_since: Option<String>,
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
    /// User-selected scoreboard window. `All` (the default) keeps the
    /// historical lifetime view. Non-`All` variants zero out snapshot-
    /// sourced fields and filter timestamp-bearing slices to the window.
    /// See [`ScoreboardWindow`] for the per-source semantics. v6+.
    pub window: ScoreboardWindow,
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
            window: ScoreboardWindow::All,
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

    // Window cutoff. `None` (i.e. `window == All`) preserves the legacy
    // lifetime behavior; `Some(since)` triggers per-source filtering and
    // skips snapshot reads (which lack per-event timestamps).
    let now_for_window = inputs.now.unwrap_or_else(Utc::now);
    let since: Option<DateTime<Utc>> = inputs.window.duration().map(|d| now_for_window - d);
    let windowed = since.is_some();

    // Snapshot reads (pr.json, task_review.json, tokens.json,
    // planning_duels.json) have no per-event timestamp, so they only run
    // for the lifetime (`All`) window. Under a windowed view they zero
    // out — the frontend renders 0 as `—` via emptyScoreboardNode().
    // TODO(phase-3+): timestamped snapshot logs would unblock real
    // windowing of these columns.
    if !windowed {
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
                summary.pr.merged_with_revision =
                    summary.pr.merged_with_revision.saturating_add(value);
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
    }

    overlay_audit_tool_calls(&mut agents, audit_tool_calls);
    overlay_audit_tool_calls_by_surface(&mut agents, inputs.audit_tool_calls_by_surface);

    // Planning-duel rows are the "who actually ran?" scoreboard projection:
    // metrics are recorded from invocation family + slot, while the stored
    // roles identify the selected family for each slot. Skipped under
    // windowed views — see the snapshot-zeroing comment above.
    let planning_duel_runs = if windowed {
        Vec::new()
    } else {
        planning_duel_scoreboard::load_runs(scoreboard_dir)?
    };
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

    overlay_knowledge_counters(&mut agents, inputs, since);
    overlay_friction_reported(&mut agents, inputs.frictions, since);

    for task in tasks {
        if matches!(task.status, TaskStatus::Done | TaskStatus::Archived)
            && in_window(task_done_at(task), since)
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
        // Windowed views filter by `created_at` so an old task created
        // outside the window doesn't get re-counted today.
        if in_window(Some(task.created_at), since) {
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
    }

    let workflows_run = aggregate_workflows_run(inputs.job_runs, since);
    let top_tools: Vec<TopToolCall> = inputs
        .top_tool_calls
        .iter()
        .map(|row| TopToolCall {
            role: row.role.clone(),
            tool_name: row.tool_name.clone(),
            count: row.total,
        })
        .collect();
    // recent_7d intentionally uses the full (unfiltered) tasks slice —
    // its 7d boundary is a fixed "is this still being used" signal,
    // independent of the user-selected `window`.
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
        window: inputs.window.as_str().to_string(),
        window_since: since.map(|t| t.to_rfc3339()),
    })
}

/// `true` when `timestamp` is at or after `since`. `since == None` means
/// the lifetime window — everything is in-window.
fn in_window(timestamp: Option<DateTime<Utc>>, since: Option<DateTime<Utc>>) -> bool {
    match (timestamp, since) {
        (_, None) => true,
        (Some(ts), Some(cut)) => ts >= cut,
        (None, Some(_)) => false,
    }
}

fn aggregate_workflows_run(runs: &[JobRun], since: Option<DateTime<Utc>>) -> Vec<WorkflowRunCount> {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for run in runs {
        if run.state == JobRunState::Success && in_window(Some(run_completed_at(run)), since) {
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
    since: Option<DateTime<Utc>>,
) {
    for learning in inputs.learnings {
        if !in_window(Some(learning.created_at), since) {
            continue;
        }
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
        if !in_window(Some(adr.created_at), since) {
            continue;
        }
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
    since: Option<DateTime<Utc>>,
) {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for stored in frictions {
        if !in_window(Some(stored.record.created_at), since) {
            continue;
        }
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
#[cfg(test)]
mod tests;
