use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use chrono::Utc;
use orbit_common::types::{
    OrbitError, PlannerSlot, Task, TaskStatus, normalize_attribution_label,
    normalize_optional_attribution_label,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::planning_duel_scoreboard;
use crate::AuditToolCallCountsByRole;
use orbit_common::utility::fs::atomic_write_text_volatile as write_atomic;

const SUMMARY_FILENAME: &str = "summary.json";
// v2 adds `task_review.threads`; v1 readers can ignore the extra field.
const CURRENT_SCHEMA_VERSION: u32 = 2;
const TASK_REVIEW_THREADS_METRIC: &str = "task-review-threads";
const LEGACY_TASK_REVIEW_MESSAGES_METRIC: &str = "task-review-messages";

type ModelScoreboard = BTreeMap<String, BTreeMap<String, u64>>;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrictionSummary {
    pub reported: u64,
    pub accepted: u64,
    pub rejected: u64,
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
pub struct AgentSummary {
    pub tasks_completed: u64,
    pub friction: FrictionSummary,
    pub tokens: TokenSummary,
    pub duels: DuelSummary,
    pub pr: PrSummary,
    #[serde(default)]
    pub task_review: TaskReviewSummary,
    pub tool_calls: u64,
    #[serde(default)]
    pub failed_tool_calls: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScoreboardSummary {
    pub schema_version: u32,
    pub generated_at: String,
    pub agents: BTreeMap<String, AgentSummary>,
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
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    total_tokens: u64,
    #[serde(default, alias = "output_tokens")]
    total_output_tokens: u64,
    #[serde(default)]
    total_tool_calls: u64,
}

pub fn generate_summary(
    scoreboard_dir: &Path,
    tasks: &[Task],
) -> Result<ScoreboardSummary, OrbitError> {
    generate_summary_with_audit_tool_calls(scoreboard_dir, tasks, &[])
}

pub fn generate_summary_with_audit_tool_calls(
    scoreboard_dir: &Path,
    tasks: &[Task],
    audit_tool_calls: &[AuditToolCallCountsByRole],
) -> Result<ScoreboardSummary, OrbitError> {
    let mut agents: BTreeMap<String, AgentSummary> = BTreeMap::new();

    let friction = read_model_scoreboard(scoreboard_dir, "friction_bounty.json")?;
    overlay_nested_metric(
        &mut agents,
        &friction,
        "issues-reported",
        |summary, value| {
            summary.friction.reported = summary.friction.reported.saturating_add(value);
        },
    );
    overlay_nested_metric(
        &mut agents,
        &friction,
        "issues-accepted",
        |summary, value| {
            summary.friction.accepted = summary.friction.accepted.saturating_add(value);
        },
    );
    overlay_nested_metric(
        &mut agents,
        &friction,
        "issues-rejected",
        |summary, value| {
            summary.friction.rejected = summary.friction.rejected.saturating_add(value);
        },
    );

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
            .map(model_key)
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

    for run in planning_duel_scoreboard::load_runs(scoreboard_dir)? {
        let planner_a = agents
            .entry(model_key(&run.roles.planner_a.model))
            .or_default();
        planner_a.duels.participated = planner_a.duels.participated.saturating_add(1);
        let planner_b = agents
            .entry(model_key(&run.roles.planner_b.model))
            .or_default();
        planner_b.duels.participated = planner_b.duels.participated.saturating_add(1);
        let arbiter = agents
            .entry(model_key(&run.roles.arbiter.model))
            .or_default();
        arbiter.duels.participated = arbiter.duels.participated.saturating_add(1);

        match run.outcome.winner {
            PlannerSlot::PlannerA => {
                let planner_a = agents
                    .entry(model_key(&run.roles.planner_a.model))
                    .or_default();
                planner_a.duels.wins = planner_a.duels.wins.saturating_add(1);
                let planner_b = agents
                    .entry(model_key(&run.roles.planner_b.model))
                    .or_default();
                planner_b.duels.losses = planner_b.duels.losses.saturating_add(1);
            }
            PlannerSlot::PlannerB => {
                let planner_b = agents
                    .entry(model_key(&run.roles.planner_b.model))
                    .or_default();
                planner_b.duels.wins = planner_b.duels.wins.saturating_add(1);
                let planner_a = agents
                    .entry(model_key(&run.roles.planner_a.model))
                    .or_default();
                planner_a.duels.losses = planner_a.duels.losses.saturating_add(1);
            }
        }
    }

    for task in tasks {
        if !matches!(task.status, TaskStatus::Done | TaskStatus::Archived) {
            continue;
        }
        let Some(model) = normalize_optional_attribution_label(
            task.model.as_deref().or(task.implemented_by.as_deref()),
            task.model.as_deref(),
        ) else {
            continue;
        };
        let summary = agents.entry(model_key(&model)).or_default();
        summary.tasks_completed = summary.tasks_completed.saturating_add(1);
    }

    Ok(ScoreboardSummary {
        schema_version: CURRENT_SCHEMA_VERSION,
        generated_at: Utc::now().to_rfc3339(),
        agents,
    })
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
) -> Result<ModelScoreboard, OrbitError> {
    let path = scoreboard_dir.join(file_name);
    if !path.exists() {
        return Ok(ModelScoreboard::new());
    }
    let raw =
        fs::read_to_string(&path).map_err(|e| OrbitError::Io(format!("read {file_name}: {e}")))?;
    if raw.trim().is_empty() {
        return Ok(ModelScoreboard::new());
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
    scoreboard: &ModelScoreboard,
    metric: &str,
    mut apply: impl FnMut(&mut AgentSummary, u64),
) {
    let Some(by_model) = scoreboard.get(metric) else {
        return;
    };

    for (model, value) in by_model {
        let summary = agents.entry(model_key(model)).or_default();
        apply(summary, *value);
    }
}

fn overlay_audit_tool_calls(
    agents: &mut BTreeMap<String, AgentSummary>,
    audit_tool_calls: &[AuditToolCallCountsByRole],
) {
    let mut by_model: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for row in audit_tool_calls {
        let model = model_key(&row.role);
        if model.is_empty() {
            continue;
        }
        let entry = by_model.entry(model).or_default();
        entry.0 = entry.0.saturating_add(row.total);
        entry.1 = entry.1.saturating_add(row.failed);
    }

    for (model, (total, failed)) in by_model {
        let summary = agents.entry(model).or_default();
        // Total competes with token scoreboard data; failures only exist in audit rows.
        summary.tool_calls = summary.tool_calls.max(total);
        summary.failed_tool_calls = summary.failed_tool_calls.saturating_add(failed);
    }
}

fn model_key(model: &str) -> String {
    normalize_attribution_label(model, None)
}

fn normalize_model_scoreboard(parsed: Value) -> Result<ModelScoreboard, OrbitError> {
    let mut normalized = ModelScoreboard::new();
    let Value::Object(metrics) = parsed else {
        return Err(OrbitError::Io(
            "scoreboard json must be an object".to_string(),
        ));
    };

    for (metric, metric_value) in metrics {
        let Value::Object(entries) = metric_value else {
            continue;
        };
        let model_entries = normalized
            .entry(canonical_scoreboard_metric(&metric).to_string())
            .or_default();
        for (first_key, first_value) in entries {
            match first_value {
                Value::Number(number) => {
                    let value = number.as_u64().ok_or_else(|| {
                        OrbitError::Io("scoreboard counter must be u64".to_string())
                    })?;
                    *model_entries.entry(first_key).or_insert(0) += value;
                }
                Value::Object(inner) => {
                    for (model, value) in inner {
                        let Value::Number(number) = value else {
                            continue;
                        };
                        let count = number.as_u64().ok_or_else(|| {
                            OrbitError::Io("scoreboard counter must be u64".to_string())
                        })?;
                        *model_entries.entry(model).or_insert(0) += count;
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

        let gpt5 = summary.agents.get("gpt-5").expect("gpt-5 summary");
        assert_eq!(gpt5.tool_calls, 3);
        assert_eq!(gpt5.failed_tool_calls, 2);
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

        let gpt5 = summary.agents.get("gpt-5").expect("gpt-5 summary");
        assert_eq!(gpt5.tokens.total, 10);
        assert_eq!(gpt5.tokens.output, 4);
        assert_eq!(gpt5.tool_calls, 5);
        assert_eq!(gpt5.failed_tool_calls, 2);
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

        let gpt5 = summary.agents.get("gpt-5").expect("gpt-5 summary");
        assert_eq!(gpt5.tokens.total, 10);
        assert_eq!(gpt5.tokens.output, 4);
        assert_eq!(gpt5.tool_calls, 7);
        assert_eq!(gpt5.failed_tool_calls, 3);
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

        assert_eq!(summary.schema_version, 2);
        let reviewer = summary
            .agents
            .get("gpt-reviewer")
            .expect("reviewer summary");
        assert_eq!(reviewer.task_review.threads, 2);
        assert_eq!(reviewer.pr.review_comments, 1);
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

        let reviewer = summary
            .agents
            .get("gpt-reviewer")
            .expect("reviewer summary");
        assert_eq!(reviewer.task_review.threads, 2);
    }
}
