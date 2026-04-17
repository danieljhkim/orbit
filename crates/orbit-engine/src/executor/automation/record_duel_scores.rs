//! `record_duel_scores` automation.
//!
//! Appends one entry to `.orbit/state/scoreboard/duel.json` per completed duel
//! run — whether the run ended with a merged PR, an approved-but-unmerged
//! PR, or a fix-loop exhaustion. Recording is unconditional by design:
//! filtering out "bad" runs would bias the scoreboard in favor of agents
//! that coincidentally dodged infrastructure flakes.
//!
//! The executor composes two layers:
//!
//! 1. [`build_duel_run`] — a pure function from `(input, now, scope)` to a
//!    [`DuelRun`]. Tested in isolation against synthetic inputs covering
//!    every outcome variant and ambiguity value.
//! 2. [`record_duel_scores`] — the thin host-coupled wrapper that derives
//!    the scope via `git diff`, computes `now`, and calls the pure function
//!    before appending to the scoreboard file.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use orbit_store::duel_scoreboard::{self, ReviewerTally};
use orbit_types::OrbitError;
use orbit_types::{
    Ambiguity, Cost, Decision, DuelRun, ImplementerStats, Outcome, PerCommentVerdict,
    ReviewerStats, RoleAssignment, Roles, Scores, Severity, TaskClass, TaskScope,
    ValidIssuesBySeverity, Verdict,
};
use serde_json::Value;

use crate::context::{RuntimeHost, TaskHost};

use super::input::{input_string_field, required_input_string};

/// Parse a required role assignment from current_input.
///
/// Each role contributes two flat fields — `<role>_agent_cli` and
/// `<role>_model` — that were populated by `select_duel_roles`. Both must
/// be present by the time the pipeline reaches `record_duel_scores`, so
/// missing values are loud errors rather than silent defaults.
fn parse_role(input: &Value, role: &str) -> Result<RoleAssignment, OrbitError> {
    let agent_key = format!("{role}_agent_cli");
    let model_key = format!("{role}_model");
    let agent = required_input_string(input, &agent_key)?.to_string();
    let model = required_input_string(input, &model_key)?.to_string();
    Ok(RoleAssignment { agent, model })
}

fn parse_decision(input: &Value) -> Result<Decision, OrbitError> {
    let raw = required_input_string(input, "decision")?;
    match raw {
        "APPROVED" => Ok(Decision::Approved),
        "REQUEST_CHANGES" => Ok(Decision::RequestChanges),
        other => Err(OrbitError::Execution(format!(
            "record_duel_scores: unexpected decision '{other}'"
        ))),
    }
}

fn parse_per_comment(input: &Value) -> Result<Vec<PerCommentVerdict>, OrbitError> {
    match input.get("per_comment") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(value) => serde_json::from_value::<Vec<PerCommentVerdict>>(value.clone())
            .map_err(|err| OrbitError::InvalidInput(format!("invalid per_comment array: {err}"))),
    }
}

fn parse_ambiguity(input: &Value) -> Result<Option<Ambiguity>, OrbitError> {
    match input.get("task_class_ambiguity") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value::<Ambiguity>(value.clone())
            .map(Some)
            .map_err(|err| {
                OrbitError::InvalidInput(format!("invalid task_class_ambiguity: {err}"))
            }),
    }
}

fn parse_float(input: &Value, key: &str) -> Result<f32, OrbitError> {
    input
        .get(key)
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .ok_or_else(|| OrbitError::InvalidInput(format!("missing numeric input.{key}")))
}

fn parse_u32(input: &Value, key: &str, default: u32) -> u32 {
    input
        .get(key)
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(default)
}

fn parse_bool(input: &Value, key: &str, default: bool) -> bool {
    input.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn parse_pr_number(input: &Value) -> Option<u64> {
    input.get("pr_number").and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    })
}

fn parse_started_at(input: &Value) -> Result<DateTime<Utc>, OrbitError> {
    let raw = required_input_string(input, "duel_started_at")?;
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| {
            OrbitError::InvalidInput(format!("duel_started_at must be RFC3339 timestamp: {err}"))
        })
}

fn implementer_stats_from_per_comment(per_comment: &[PerCommentVerdict]) -> ImplementerStats {
    let mut hist = ValidIssuesBySeverity {
        high: 0,
        medium: 0,
        low: 0,
    };
    for row in per_comment {
        if row.verdict != Verdict::Valid {
            continue;
        }
        match row.severity {
            Some(Severity::High) => hist.high += 1,
            Some(Severity::Medium) => hist.medium += 1,
            Some(Severity::Low) => hist.low += 1,
            // A valid comment without severity contributes to the count
            // of valid issues but not to the severity histogram. This is
            // intentional — severity is optional in ArbiterVerdict.
            None => {}
        }
    }
    ImplementerStats {
        valid_issues_against: hist,
    }
}

fn reviewer_stats_from_per_comment(per_comment: &[PerCommentVerdict]) -> ReviewerStats {
    let verdicts: Vec<Verdict> = per_comment.iter().map(|p| p.verdict).collect();
    let tally: ReviewerTally = duel_scoreboard::tally_reviewer_stats(&verdicts);
    ReviewerStats {
        total_comments: tally.total_comments,
        valid: tally.valid,
        invalid: tally.invalid,
        out_of_scope: tally.out_of_scope,
        nitpick: tally.nitpick,
        precision: tally.precision,
        arbiter_override_rate: tally.arbiter_override_rate,
    }
}

fn generate_run_id(now: DateTime<Utc>) -> String {
    now.format("R%Y%m%d-%H%M%S-%3f").to_string()
}

/// Pure constructor: given the already-piped current_input, a "now"
/// timestamp, and the pre-computed task scope, produce a fully-populated
/// [`DuelRun`] ready to be appended to the scoreboard.
///
/// Exposed at crate visibility for unit tests.
pub(crate) fn build_duel_run(
    input: &Value,
    now: DateTime<Utc>,
    scope: TaskScope,
) -> Result<DuelRun, OrbitError> {
    let task_id = required_input_string(input, "task_id")?.to_string();
    let started_at = parse_started_at(input)?;

    let roles = Roles {
        implementer: parse_role(input, "implementer")?,
        reviewer: parse_role(input, "reviewer")?,
        arbiter: parse_role(input, "arbiter")?,
    };

    let decision = parse_decision(input)?;
    let per_comment = parse_per_comment(input)?;
    let ambiguity = parse_ambiguity(input)?;
    let reviewer_score = parse_float(input, "reviewer_score")?;
    let implementer_score = parse_float(input, "implementer_score")?;

    let fix_loop_iterations = parse_u32(input, "fix_loop_iterations", 0);
    let fix_loop_exhausted = parse_bool(input, "fix_loop_exhausted", false);
    let merged = parse_bool(input, "merged", false);
    let pr_number = parse_pr_number(input);

    let wall_clock_seconds = now.signed_duration_since(started_at).num_seconds().max(0) as u64;

    Ok(DuelRun {
        run_id: generate_run_id(now),
        task_id,
        completed_at: now,
        task_class: TaskClass {
            scope,
            ambiguity,
            source: "derived".to_string(),
        },
        roles,
        outcome: Outcome {
            decision,
            fix_loop_iterations,
            fix_loop_exhausted,
            pr_number,
            merged,
        },
        scores: Scores {
            implementer_score,
            reviewer_score,
        },
        reviewer_stats: reviewer_stats_from_per_comment(&per_comment),
        implementer_stats: implementer_stats_from_per_comment(&per_comment),
        cost: Cost {
            wall_clock_seconds,
            tokens_in: None,
            tokens_out: None,
        },
    })
}

/// Resolve the scope for this run. Falls back to [`TaskScope::Other`]
/// when the input does not provide base/head refs (which may happen on
/// degenerate pipeline paths where no git history is available). The
/// spec intentionally treats "cannot compute" as a non-fatal classification
/// rather than blocking the scoreboard write.
fn resolve_scope<H: RuntimeHost + ?Sized>(host: &H, input: &Value) -> TaskScope {
    let repo_root = match host.repo_root() {
        Ok(path) => PathBuf::from(path),
        Err(_) => return TaskScope::Other,
    };
    let base = input_string_field(input, "base_ref");
    let head = input_string_field(input, "head_ref");
    match (base, head) {
        (Some(base), Some(head)) => {
            duel_scoreboard::derive_task_scope(&repo_root, &base, &head).unwrap_or(TaskScope::Other)
        }
        _ => TaskScope::Other,
    }
}

pub(super) fn record_duel_scores<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let now = Utc::now();
    let scope = resolve_scope(host, input);
    let run = build_duel_run(input, now, scope)?;
    duel_scoreboard::append_run(host.scoreboard_dir(), &run)?;
    Ok(serde_json::json!({
        "run_id": run.run_id,
        "recorded": true,
    }))
}
