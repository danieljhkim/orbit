use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use chrono::Utc;
use orbit_types::{OrbitError, PlannerSlot, Task, TaskStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::fs_utils::write_atomic;
use super::planning_duel_scoreboard;

const SUMMARY_FILENAME: &str = "summary.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;

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
pub struct AgentSummary {
    pub tasks_completed: u64,
    pub friction: FrictionSummary,
    pub tokens: TokenSummary,
    pub duels: DuelSummary,
    pub pr: PrSummary,
    pub tool_calls: u64,
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
    model: String,
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

    for token_row in read_token_agents(scoreboard_dir)? {
        let summary = agents.entry(model_key(&token_row.model)).or_default();
        summary.tokens.total = summary.tokens.total.saturating_add(token_row.total_tokens);
        summary.tokens.output = summary
            .tokens
            .output
            .saturating_add(token_row.total_output_tokens);
        summary.tool_calls = summary
            .tool_calls
            .saturating_add(token_row.total_tool_calls);
    }

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
        let Some(model) = task.implemented_by.as_deref() else {
            continue;
        };
        let summary = agents.entry(model_key(model)).or_default();
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

fn model_key(model: &str) -> String {
    model.trim().to_string()
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
        let model_entries = normalized.entry(metric).or_default();
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
