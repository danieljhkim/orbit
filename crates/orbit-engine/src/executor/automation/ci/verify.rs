//! `verify_candidate_ci` — did the repair actually go green on GitHub?
//!
//! This is the half of CI remediation that cannot be asked of the agent at
//! all: the candidate commit does not exist while `agent_implement` is
//! running. It exists after `git_push`, which is where this stage runs — after
//! publication and *before* promotion, so a bundle is only ever promoted on a
//! verified green candidate.
//!
//! Every unsettled state stays distinguishable from red. A queued workflow, a
//! running one, a cancelled one, a workflow that never appeared, and a wait
//! that ran out of budget are five different facts, and none of them is a CI
//! failure. Collapsing any of them into "red" — or, worse, into "green" —
//! would make the verdict useless.
//!
//! A green candidate returns and the pipeline promotes. Anything else moves the
//! bundle to `blocked`, records the whole structured verdict as a durable task
//! comment, and fails the step — which is what makes "promotes only when green"
//! structural rather than a condition someone could reorder away.
//!
//! Feeding a red candidate's new failure logs back into another repair
//! iteration is deliberately out of scope; the result carries the evidence
//! that iteration would need so it can be added without reshaping this stage.

use std::collections::BTreeMap;

use chrono::Utc;
use orbit_common::OrbitError;
use orbit_types::task::TaskComment;
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate};

use super::query::{CiQueries, LogScope};
use super::{
    CI_CANDIDATE_NOT_GREEN_EVENT, block_tasks, bounded_u64, optional_input_string,
    task_ids_from_input,
};

const DEFAULT_MAX_WAIT_SECONDS: u64 = 1_800;
const MAX_MAX_WAIT_SECONDS: u64 = 7_200;
const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 30;
const MAX_POLL_INTERVAL_SECONDS: u64 = 300;
const MIN_POLL_INTERVAL_SECONDS: u64 = 5;
const DEFAULT_RUNS_LIMIT: u64 = 30;
const MAX_RUNS_LIMIT: u64 = 100;
const DEFAULT_LOG_MAX_BYTES: u64 = 16_384;
const MAX_LOG_MAX_BYTES: u64 = 262_144;
/// Red runs whose logs are attached to the verdict. Enough to diagnose a
/// regression, bounded so a broadly red candidate cannot produce an unbounded
/// task note or step output.
const MAX_RED_INVESTIGATIONS: usize = 3;

/// Conclusions that mean the candidate is red. `cancelled` is handled
/// separately: it is not a pass, but it is not a failing test either.
const RED_CONCLUSIONS: &[&str] = &["failure", "timed_out", "action_required", "startup_failure"];
/// Conclusions that do not block promotion. `skipped` and `neutral` are how
/// GitHub reports a check that ran its gate and declined to fail.
const GREEN_CONCLUSIONS: &[&str] = &["success", "skipped", "neutral"];

/// Consuming the wait budget, behind a seam so the timeout path is testable
/// without spending wall-clock time.
pub(super) trait Waiter {
    /// Wait, and report the seconds consumed from the budget.
    fn wait(&self, seconds: u64) -> u64;
}

pub(super) struct RealWaiter;

impl Waiter for RealWaiter {
    fn wait(&self, seconds: u64) -> u64 {
        std::thread::sleep(std::time::Duration::from_secs(seconds));
        seconds
    }
}

struct Bounds {
    max_wait_seconds: u64,
    poll_interval_seconds: u64,
    runs_limit: u64,
    log_max_bytes: usize,
}

/// What the last poll saw, before it is turned into a verdict.
#[derive(Default)]
struct Observation {
    latest_by_workflow: BTreeMap<String, Value>,
    missing_workflows: Vec<String>,
}

impl Observation {
    fn by_state<'a>(&'a self, predicate: impl Fn(&Value) -> bool + 'a) -> Vec<Value> {
        self.latest_by_workflow
            .values()
            .filter(|run| predicate(run))
            .cloned()
            .collect()
    }

    /// The least-advanced thing still outstanding, or `None` when settled.
    ///
    /// Ordered least-advanced first so the reported reason is the one furthest
    /// from a verdict: a workflow that never appeared explains the wait better
    /// than one that is merely still running.
    fn pending_state(&self) -> Option<&'static str> {
        if !self.missing_workflows.is_empty() || self.latest_by_workflow.is_empty() {
            return Some("missing");
        }
        if self
            .latest_by_workflow
            .values()
            .any(|run| status(run) == Some("queued"))
        {
            return Some("queued");
        }
        if self
            .latest_by_workflow
            .values()
            .any(|run| status(run) != Some("completed"))
        {
            return Some("in_progress");
        }
        None
    }
}

fn status(run: &Value) -> Option<&str> {
    run.get("status").and_then(Value::as_str)
}

fn conclusion(run: &Value) -> Option<&str> {
    run.get("conclusion").and_then(Value::as_str)
}

pub(super) fn verify<H: RuntimeHost + ?Sized, Q: CiQueries + ?Sized, W: Waiter + ?Sized>(
    host: &H,
    queries: &Q,
    waiter: &W,
    input: &Value,
) -> Result<Value, OrbitError> {
    let candidate_sha = required_candidate_sha(input)?;
    let head_branch = optional_input_string(input, "head_branch")
        .or_else(|| optional_input_string(input, "head"))
        .or_else(|| optional_input_string(input, "branch"))
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "verify_candidate_ci requires input.head_branch naming the published branch"
                    .to_string(),
            )
        })?;
    let expected_workflows = expected_workflows(input);
    let bounds = Bounds {
        max_wait_seconds: bounded_u64(
            input,
            "max_wait_seconds",
            DEFAULT_MAX_WAIT_SECONDS,
            MAX_MAX_WAIT_SECONDS,
        )?,
        poll_interval_seconds: bounded_u64(
            input,
            "poll_interval_seconds",
            DEFAULT_POLL_INTERVAL_SECONDS,
            MAX_POLL_INTERVAL_SECONDS,
        )?
        .max(MIN_POLL_INTERVAL_SECONDS),
        runs_limit: bounded_u64(input, "runs_limit", DEFAULT_RUNS_LIMIT, MAX_RUNS_LIMIT)?,
        log_max_bytes: bounded_u64(
            input,
            "log_max_bytes",
            DEFAULT_LOG_MAX_BYTES,
            MAX_LOG_MAX_BYTES,
        )? as usize,
    };

    let mut waited = 0u64;
    let mut polls = 0u64;
    let (verdict, pending_state, observation) = loop {
        polls += 1;
        let observation = observe(
            queries,
            &head_branch,
            &candidate_sha,
            &expected_workflows,
            bounds.runs_limit,
        )?;
        match observation.pending_state() {
            None => break (settled_verdict(&observation), None, observation),
            Some(pending) => {
                if waited >= bounds.max_wait_seconds {
                    // A wait that ran out of budget is not a CI failure. Say
                    // which unsettled state we ran out on.
                    let verdict = if bounds.max_wait_seconds == 0 {
                        pending
                    } else {
                        "wait_timeout"
                    };
                    break (verdict, Some(pending), observation);
                }
                let slice = bounds
                    .poll_interval_seconds
                    .min(bounds.max_wait_seconds - waited);
                waited += waiter.wait(slice);
            }
        }
    };

    let red = observation.by_state(|run| RED_CONCLUSIONS.contains(&conclusion(run).unwrap_or("")));
    let cancelled = observation.by_state(|run| conclusion(run) == Some("cancelled"));
    let queued = observation.by_state(|run| status(run) == Some("queued"));
    let in_progress = observation.by_state(|run| {
        status(run).is_some_and(|status| status != "completed" && status != "queued")
    });
    let promotable = verdict == "green";

    let failure_evidence = if red.is_empty() {
        Vec::new()
    } else {
        red_evidence(queries, &red, bounds.log_max_bytes)
    };

    let mut result = json!({
        "phase": "verify_candidate_ci",
        "verdict": verdict,
        "pending_state": pending_state,
        "promotable": promotable,
        "candidate_sha": candidate_sha,
        "head_branch": head_branch,
        "expected_workflows": expected_workflows,
        "observed_workflows": observation.latest_by_workflow.keys().collect::<Vec<_>>(),
        "missing_workflows": observation.missing_workflows,
        "checks": observation.latest_by_workflow.values().collect::<Vec<_>>(),
        "red": red,
        "cancelled": cancelled,
        "queued": queued,
        "in_progress": in_progress,
        "failure_evidence": failure_evidence,
        "waited_seconds": waited,
        "polls": polls,
        "max_wait_seconds": bounds.max_wait_seconds,
        "blocked_task_ids": Vec::<String>::new(),
    });

    if promotable {
        return Ok(result);
    }

    // Not green. Record the whole verdict where a human will find it, block
    // the bundle, and fail the step so promotion is unreachable rather than
    // merely skipped.
    let task_ids = task_ids_from_input(input);
    let headline = format!(
        "Candidate {candidate_sha} on '{head_branch}' was not verified green: verdict={verdict}\
         {pending}. Observed {observed} affected workflow run(s) after {waited}s and {polls} \
         poll(s); {red_count} red, {cancelled_count} cancelled, {queued_count} queued, \
         {running_count} still running, {missing_count} never appeared. A wait timeout and a \
         cancelled run are not CI failures, and none of these states is a pass.",
        pending = pending_state
            .map(|pending| format!(" (pending_state={pending})"))
            .unwrap_or_default(),
        observed = result["observed_workflows"].as_array().map_or(0, Vec::len),
        red_count = result["red"].as_array().map_or(0, Vec::len),
        cancelled_count = result["cancelled"].as_array().map_or(0, Vec::len),
        queued_count = result["queued"].as_array().map_or(0, Vec::len),
        running_count = result["in_progress"].as_array().map_or(0, Vec::len),
        missing_count = result["missing_workflows"].as_array().map_or(0, Vec::len),
    );
    result["detail"] = json!(headline);
    record_verdict(host, &task_ids, &headline, &result)?;
    result["blocked_task_ids"] = json!(block_tasks(
        host,
        &task_ids,
        CI_CANDIDATE_NOT_GREEN_EVENT,
        &headline
    )?);
    Err(OrbitError::Execution(headline))
}

/// Durable home for the full structured verdict.
///
/// The status note is capped — it has to be, a note is read in a list — so the
/// bounded headline goes there and the whole result goes here, as a comment,
/// the same way a failed PR handoff records its diagnostics. The verdict is
/// shaped so a later bounded repair iteration can consume it directly;
/// building that iteration is deliberately not this stage's job.
fn record_verdict<H: RuntimeHost + ?Sized>(
    host: &H,
    task_ids: &[String],
    headline: &str,
    result: &Value,
) -> Result<(), OrbitError> {
    let message = format!(
        "{VERDICT_COMMENT_HEADER}\n\n{headline}\n\n{}",
        serde_json::to_string_pretty(result)
            .unwrap_or_else(|error| format!("<verdict could not be serialized: {error}>")),
    );
    for task_id in task_ids {
        if host
            .get_task_comments(task_id)?
            .iter()
            .any(|comment| comment.message.starts_with(VERDICT_COMMENT_HEADER))
        {
            continue;
        }
        host.apply_task_automation_update(
            task_id,
            TaskAutomationUpdate {
                append_comments: vec![TaskComment {
                    at: Utc::now(),
                    by: VERDICT_COMMENT_ACTOR.to_string(),
                    message: message.clone(),
                }],
                ..TaskAutomationUpdate::default()
            },
        )?;
    }
    Ok(())
}

const VERDICT_COMMENT_HEADER: &str = "candidate CI verification failed";
const VERDICT_COMMENT_ACTOR: &str = "system";

fn settled_verdict(observation: &Observation) -> &'static str {
    if observation
        .latest_by_workflow
        .values()
        .any(|run| RED_CONCLUSIONS.contains(&conclusion(run).unwrap_or("")))
    {
        return "red";
    }
    if observation
        .latest_by_workflow
        .values()
        .any(|run| conclusion(run) == Some("cancelled"))
    {
        return "cancelled";
    }
    if observation
        .latest_by_workflow
        .values()
        .all(|run| GREEN_CONCLUSIONS.contains(&conclusion(run).unwrap_or("")))
    {
        return "green";
    }
    // A completed run with a conclusion nobody here recognises is not a pass.
    "red"
}

/// One poll: the affected runs on the exact candidate SHA, latest per workflow.
///
/// Filtering on `reported_head_sha` is what makes this "the exact candidate
/// commit" rather than "whatever ran on that branch". Every workflow run on
/// that SHA is affected, which is how informational checks are covered
/// alongside required ones — GitHub does not mark the difference here, and a
/// stage that only watched required checks would report a red informational
/// job as green.
fn observe<Q: CiQueries + ?Sized>(
    queries: &Q,
    head_branch: &str,
    candidate_sha: &str,
    expected_workflows: &[String],
    runs_limit: u64,
) -> Result<Observation, OrbitError> {
    let runs = queries.runs_for_branch(head_branch, runs_limit)?;
    let mut latest_by_workflow: BTreeMap<String, Value> = BTreeMap::new();
    for run in runs {
        if run.get("reported_head_sha").and_then(Value::as_str) != Some(candidate_sha) {
            continue;
        }
        let Some(workflow) = run
            .get("workflow")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        else {
            continue;
        };
        let newer = latest_by_workflow
            .get(&workflow)
            .is_none_or(|known| order(&run) > order(known));
        if newer {
            latest_by_workflow.insert(workflow, run);
        }
    }
    let missing_workflows = expected_workflows
        .iter()
        .filter(|workflow| !latest_by_workflow.contains_key(*workflow))
        .cloned()
        .collect();
    Ok(Observation {
        latest_by_workflow,
        missing_workflows,
    })
}

fn order(run: &Value) -> (String, u64) {
    (
        run.get("created_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        run.get("run_id").and_then(Value::as_u64).unwrap_or(0),
    )
}

fn red_evidence<Q: CiQueries + ?Sized>(
    queries: &Q,
    red: &[Value],
    log_max_bytes: usize,
) -> Vec<Value> {
    red.iter()
        .take(MAX_RED_INVESTIGATIONS)
        .filter_map(|run| {
            let run_id = run.get("run_id").and_then(Value::as_u64)?.to_string();
            let mut entry = json!({
                "run_id": run.get("run_id"),
                "workflow": run.get("workflow"),
                "url": run.get("url"),
                "conclusion": run.get("conclusion"),
            });
            if let Ok(view) = queries.run_view(&run_id) {
                entry["failed_jobs"] = view.get("failed_jobs").cloned().unwrap_or(json!([]));
            }
            if let Ok(log) = queries.run_logs(&run_id, LogScope::Failed, log_max_bytes) {
                entry["log_excerpt"] = json!(log.text);
                entry["log_truncated"] = json!(log.truncated);
                entry["log_total_bytes"] = json!(log.total_bytes);
            }
            Some(entry)
        })
        .collect()
}

fn required_candidate_sha(input: &Value) -> Result<String, OrbitError> {
    let sha = optional_input_string(input, "candidate_sha")
        .or_else(|| optional_input_string(input, "local_sha"))
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "verify_candidate_ci requires input.candidate_sha naming the published commit"
                    .to_string(),
            )
        })?;
    if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(OrbitError::InvalidInput(format!(
            "verify_candidate_ci requires an exact 40- or 64-character commit sha, got '{sha}'"
        )));
    }
    Ok(sha)
}

fn expected_workflows(input: &Value) -> Vec<String> {
    let mut workflows: Vec<String> = input
        .get("expected_workflows")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    workflows.sort();
    workflows.dedup();
    workflows
}
