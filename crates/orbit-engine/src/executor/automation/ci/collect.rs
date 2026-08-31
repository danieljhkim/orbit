//! `collect_ci_evidence` — the host-owned CI discovery stage.
//!
//! Runs before any agent is launched, so the credentials it needs never have
//! to cross into a sandbox. What crosses instead is one bounded, redacted
//! snapshot: failed jobs and steps, log excerpts, the event-reported SHA and
//! the commit the runner actually checked out as separate fields, and the
//! stale/superseding evidence that says which failures are still real.
//!
//! Losing the agent's ability to ask a follow-up question mid-diagnosis is the
//! accepted cost of that boundary. The compensation is that the snapshot is
//! generous and that every bound it hit is reported in `truncation` — a
//! reader must never have to guess whether "no more failures" meant "none" or
//! "we stopped looking".

use orbit_common::OrbitError;
use serde_json::{Value, json};

use super::query::{CiQueries, LogScope};
use super::{
    OUTCOME_CAPABILITY_UNAVAILABLE, OUTCOME_CURRENT_FAILURES, OUTCOME_NO_CURRENT_FAILURE,
    bounded_u64, optional_input_string, unsuccessful_conclusion,
};

/// Snapshot schema version. Bump when a consumer would misread an older
/// snapshot; `file_ci_failure_tasks` reads this field before anything else.
pub(super) const CI_EVIDENCE_SCHEMA_VERSION: u64 = 1;

const DEFAULT_RUNS_PER_REF: u64 = 20;
const MAX_RUNS_PER_REF: u64 = 100;
const DEFAULT_MAX_PULL_REQUESTS: u64 = 10;
const MAX_PULL_REQUESTS: u64 = 50;
const DEFAULT_MAX_INVESTIGATED_RUNS: u64 = 6;
const MAX_INVESTIGATED_RUNS: u64 = 25;
const DEFAULT_LOG_MAX_BYTES: u64 = 16_384;
const MAX_LOG_MAX_BYTES: u64 = 262_144;
/// Cap on full-log reads taken purely to evidence a checkout commit. The
/// failed-step log usually lacks it, and a full log can be tens of megabytes.
const DEFAULT_MAX_CHECKOUT_LOG_READS: u64 = 3;

/// Which of the workspace's heads a run belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefKind {
    Integration,
    Release,
    PullRequest,
}

impl RefKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Integration => "integration",
            Self::Release => "release",
            Self::PullRequest => "pull_request",
        }
    }
}

/// One head to scan, with the SHA it currently points at.
struct ScannedRef {
    kind: RefKind,
    branch: String,
    head_sha: Option<String>,
    pr_number: Option<Value>,
    pr_url: Option<Value>,
}

struct Bounds {
    runs_per_ref: u64,
    max_pull_requests: u64,
    max_investigated_runs: usize,
    log_max_bytes: usize,
    max_checkout_log_reads: usize,
}

fn bounds_from_input(input: &Value) -> Result<Bounds, OrbitError> {
    Ok(Bounds {
        runs_per_ref: bounded_u64(
            input,
            "runs_per_ref",
            DEFAULT_RUNS_PER_REF,
            MAX_RUNS_PER_REF,
        )?,
        max_pull_requests: bounded_u64(
            input,
            "max_pull_requests",
            DEFAULT_MAX_PULL_REQUESTS,
            MAX_PULL_REQUESTS,
        )?,
        max_investigated_runs: bounded_u64(
            input,
            "max_investigated_runs",
            DEFAULT_MAX_INVESTIGATED_RUNS,
            MAX_INVESTIGATED_RUNS,
        )? as usize,
        log_max_bytes: bounded_u64(
            input,
            "log_max_bytes",
            DEFAULT_LOG_MAX_BYTES,
            MAX_LOG_MAX_BYTES,
        )? as usize,
        max_checkout_log_reads: bounded_u64(
            input,
            "max_checkout_log_reads",
            DEFAULT_MAX_CHECKOUT_LOG_READS,
            MAX_INVESTIGATED_RUNS,
        )? as usize,
    })
}

/// Collect one CI evidence snapshot.
pub(super) fn collect<Q: CiQueries + ?Sized>(
    queries: &Q,
    input: &Value,
) -> Result<Value, OrbitError> {
    let bounds = bounds_from_input(input)?;
    let auth = queries.auth_status();
    if !auth.usable() {
        // Stop here on purpose. Every later field would be an empty list that
        // reads exactly like "nothing is failing", and that conclusion
        // requires queries this host could not run.
        return Ok(json!({
            "schema_version": CI_EVIDENCE_SCHEMA_VERSION,
            "collected": false,
            "outcome_hint": OUTCOME_CAPABILITY_UNAVAILABLE,
            "capability": auth.to_json(),
            "collected_at": chrono::Utc::now().to_rfc3339(),
        }));
    }

    let repository = queries.repo_view()?;
    let default_branch = repository
        .get("default_branch")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let mut notes: Vec<String> = Vec::new();
    let mut query_errors: Vec<Value> = Vec::new();

    let refs = derive_refs(
        queries,
        input,
        default_branch.as_deref(),
        &bounds,
        &mut notes,
        &mut query_errors,
    )?;

    let mut current: Vec<Value> = Vec::new();
    let mut stale: Vec<Value> = Vec::new();
    let mut in_flight: Vec<Value> = Vec::new();
    // This query is deliberately repository-wide. Enumerating the integration,
    // release, and open-PR refs separately lets an unchanged old ref keep an
    // obsolete failure alive forever. One repository-wide list lets us choose
    // exactly one latest run for every workflow represented in the snapshot.
    let runs = match queries.repository_runs(bounds.runs_per_ref) {
        Ok(runs) => runs,
        Err(error) => {
            query_errors.push(json!({
                "query": "run_list",
                "error": error.to_string(),
            }));
            Vec::new()
        }
    };
    let runs_listed = runs.len();
    if runs_listed as u64 == bounds.runs_per_ref {
        notes.push(format!(
            "repository-wide workflow runs were listed at the cap ({}); older workflows may be absent",
            bounds.runs_per_ref
        ));
    }
    partition_runs(&refs, &runs, &mut current, &mut stale, &mut in_flight);

    sort_current_failures(&mut current);
    let discovered = current.len();
    let investigated = discovered.min(bounds.max_investigated_runs);
    if discovered > investigated {
        notes.push(format!(
            "{} of {discovered} current failures were listed but not investigated \
             (max_investigated_runs={investigated}); their run URLs are still present",
            discovered - investigated
        ));
    }
    let mut checkout_log_reads = 0usize;
    for (index, failure) in current.iter_mut().enumerate() {
        if index >= investigated {
            failure["investigated"] = json!(false);
            continue;
        }
        investigate(
            queries,
            failure,
            &bounds,
            &mut checkout_log_reads,
            &mut query_errors,
        );
    }

    Ok(json!({
        "schema_version": CI_EVIDENCE_SCHEMA_VERSION,
        "collected": true,
        "outcome_hint": if current.is_empty() { OUTCOME_NO_CURRENT_FAILURE } else { OUTCOME_CURRENT_FAILURES },
        "capability": auth.to_json(),
        "repository": repository,
        "heads": refs.iter().map(head_json).collect::<Vec<_>>(),
        "current_failures": current,
        "stale_or_superseded": stale,
        "in_flight": in_flight,
        "query_errors": query_errors,
        "truncation": json!({
            "refs_scanned": refs.len(),
            "runs_listed": runs_listed,
            "runs_per_ref": bounds.runs_per_ref,
            "pull_requests_scanned": refs
                .iter()
                .filter(|scanned| scanned.kind == RefKind::PullRequest)
                .count(),
            "max_pull_requests": bounds.max_pull_requests,
            "current_failures_discovered": discovered,
            "current_failures_investigated": investigated,
            "log_max_bytes": bounds.log_max_bytes,
            "checkout_log_reads": checkout_log_reads,
            "max_checkout_log_reads": bounds.max_checkout_log_reads,
            "notes": notes,
        }),
        "collected_at": chrono::Utc::now().to_rfc3339(),
    }))
}

/// Work out which heads to scan.
///
/// The integration branch comes from the run's own base branch — the branch
/// this workspace actually ships onto — and the release branch from what
/// GitHub reports as the repository default. Neither is guessed from a naming
/// convention, and when the two coincide the ref is scanned once.
fn derive_refs<Q: CiQueries + ?Sized>(
    queries: &Q,
    input: &Value,
    default_branch: Option<&str>,
    bounds: &Bounds,
    notes: &mut Vec<String>,
    query_errors: &mut Vec<Value>,
) -> Result<Vec<ScannedRef>, OrbitError> {
    let integration = optional_input_string(input, "integration_branch")
        .or_else(|| optional_input_string(input, "base_branch"))
        .or_else(|| default_branch.map(ToOwned::to_owned));
    let mut refs: Vec<ScannedRef> = Vec::new();

    for (kind, branch) in [
        (RefKind::Integration, integration),
        (RefKind::Release, default_branch.map(ToOwned::to_owned)),
    ] {
        let Some(branch) = branch else {
            notes.push(format!(
                "no {} branch could be derived; that head was not scanned",
                kind.as_str()
            ));
            continue;
        };
        if refs.iter().any(|scanned| scanned.branch == branch) {
            continue;
        }
        let head_sha = match queries.remote_branch_head(&branch) {
            Ok(head) => head,
            Err(error) => {
                query_errors.push(json!({
                    "query": "remote_branch_head",
                    "branch": branch,
                    "error": error.to_string(),
                }));
                None
            }
        };
        if head_sha.is_none() {
            notes.push(format!(
                "{} branch '{branch}' has no head on origin; failures there cannot be \
                 compared against a current head",
                kind.as_str()
            ));
        }
        refs.push(ScannedRef {
            kind,
            branch,
            head_sha,
            pr_number: None,
            pr_url: None,
        });
    }

    match queries.open_pull_requests(bounds.max_pull_requests) {
        Ok(pull_requests) => {
            if pull_requests.len() as u64 == bounds.max_pull_requests {
                notes.push(format!(
                    "open pull requests were listed at the cap ({}); further open \
                     pull-request heads may exist and were not scanned",
                    bounds.max_pull_requests
                ));
            }
            for pull_request in pull_requests {
                let Some(branch) = pull_request
                    .get("head_branch")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    continue;
                };
                // A head already scanned as the integration or release branch
                // would otherwise be listed twice, reporting each of its
                // failures twice.
                if refs.iter().any(|scanned| scanned.branch == branch) {
                    continue;
                }
                refs.push(ScannedRef {
                    kind: RefKind::PullRequest,
                    branch: branch.to_string(),
                    head_sha: pull_request
                        .get("reported_head_sha")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    pr_number: pull_request.get("number").cloned(),
                    pr_url: pull_request.get("url").cloned(),
                });
            }
        }
        Err(error) => {
            query_errors.push(json!({
                "query": "pr_list",
                "error": error.to_string(),
            }));
            notes.push(
                "open pull requests could not be listed; no pull-request head was scanned"
                    .to_string(),
            );
        }
    }

    Ok(refs)
}

fn head_json(scanned: &ScannedRef) -> Value {
    json!({
        "kind": scanned.kind.as_str(),
        "branch": scanned.branch,
        "current_head_sha": scanned.head_sha,
        "pr_number": scanned.pr_number,
        "pr_url": scanned.pr_url,
    })
}

/// Evaluate exactly the latest repository-wide run for each workflow.
///
/// Older unsuccessful runs remain useful evidence, but they can never become
/// current merely because the ref they ran on has not advanced. The latest run
/// supersedes them regardless of branch, SHA, status, or conclusion.
fn partition_runs(
    refs: &[ScannedRef],
    runs: &[Value],
    current: &mut Vec<Value>,
    stale: &mut Vec<Value>,
    in_flight: &mut Vec<Value>,
) {
    let mut workflows = std::collections::BTreeMap::<String, Vec<&Value>>::new();
    for run in runs {
        let workflow = run
            .get("workflow")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        workflows.entry(workflow).or_default().push(run);
    }

    for workflow_runs in workflows.values_mut() {
        workflow_runs.sort_by_key(|run| std::cmp::Reverse(run_order(run)));
        let Some(latest) = workflow_runs.first().copied() else {
            continue;
        };

        for older in workflow_runs.iter().skip(1).copied() {
            let completed = older.get("status").and_then(Value::as_str) == Some("completed");
            let conclusion = older.get("conclusion").and_then(Value::as_str);
            if completed && unsuccessful_conclusion(conclusion) {
                let mut entry = run_summary(ref_for_run(refs, older), older);
                entry["reason"] = json!("superseded_by_newer_workflow_run");
                entry["evidence"] = json!(format!(
                    "newer run {} at {} is {} with conclusion {}",
                    latest
                        .get("run_id")
                        .and_then(Value::as_u64)
                        .map_or_else(|| "unknown".to_string(), |id| id.to_string()),
                    latest
                        .get("created_at")
                        .and_then(Value::as_str)
                        .unwrap_or("an unknown time"),
                    latest
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("in an unknown state"),
                    latest
                        .get("conclusion")
                        .and_then(Value::as_str)
                        .unwrap_or("not yet completed"),
                ));
                entry["superseded_by"] = json!({
                    "run_id": latest.get("run_id"),
                    "url": latest.get("url"),
                    "created_at": latest.get("created_at"),
                    "status": latest.get("status"),
                    "conclusion": latest.get("conclusion"),
                });
                stale.push(entry);
            }
        }

        let completed = latest.get("status").and_then(Value::as_str) == Some("completed");
        let summary = run_summary(ref_for_run(refs, latest), latest);
        if !completed {
            in_flight.push(summary);
        } else if unsuccessful_conclusion(latest.get("conclusion").and_then(Value::as_str)) {
            current.push(summary);
        }
    }
}

/// Sortable position of a run: creation time first, run id as the tiebreak.
fn run_order(run: &Value) -> (String, u64) {
    (
        run.get("created_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        run.get("run_id").and_then(Value::as_u64).unwrap_or(0),
    )
}

fn ref_for_run<'a>(refs: &'a [ScannedRef], run: &Value) -> Option<&'a ScannedRef> {
    let branch = run.get("head_branch").and_then(Value::as_str)?;
    refs.iter().find(|scanned| scanned.branch == branch)
}

fn run_summary(scanned: Option<&ScannedRef>, run: &Value) -> Value {
    json!({
        "run_id": run.get("run_id"),
        "workflow": run.get("workflow"),
        "title": run.get("title"),
        "status": run.get("status"),
        "conclusion": run.get("conclusion"),
        "event": run.get("event"),
        "url": run.get("url"),
        "created_at": run.get("created_at"),
        "head_branch": run.get("head_branch"),
        "ref_kind": scanned.map(|scanned| scanned.kind.as_str()).unwrap_or("other"),
        "pr_number": scanned.and_then(|scanned| scanned.pr_number.clone()),
        "pr_url": scanned.and_then(|scanned| scanned.pr_url.clone()),
        // Three commits that are routinely conflated and are kept apart here:
        // what the event reported, what the ref points at now, and — filled in
        // by `investigate` — what the runner actually checked out.
        "event_reported_head_sha": run.get("reported_head_sha"),
        "current_ref_head_sha": scanned.and_then(|scanned| scanned.head_sha.clone()),
        "actual_checkout_shas": Value::Array(Vec::new()),
        "investigated": false,
    })
}

/// Integration first, then release, then pull requests; newest run first
/// within each. Investigation budget therefore lands on the heads that gate
/// delivery before it lands on a pull request.
fn sort_current_failures(failures: &mut [Value]) {
    let rank = |value: &Value| match value.get("ref_kind").and_then(Value::as_str) {
        Some("integration") => 0,
        Some("release") => 1,
        _ => 2,
    };
    failures.sort_by(|left, right| {
        rank(left)
            .cmp(&rank(right))
            .then_with(|| run_order(right).cmp(&run_order(left)))
    });
}

/// Fill one current failure in with its failed jobs, steps, bounded log, and
/// the commit its runner actually checked out.
fn investigate<Q: CiQueries + ?Sized>(
    queries: &Q,
    failure: &mut Value,
    bounds: &Bounds,
    checkout_log_reads: &mut usize,
    query_errors: &mut Vec<Value>,
) {
    let Some(run_id) = failure
        .get("run_id")
        .and_then(Value::as_u64)
        .map(|id| id.to_string())
    else {
        return;
    };
    failure["investigated"] = json!(true);

    match queries.run_view(&run_id) {
        Ok(view) => {
            failure["failed_jobs"] = view.get("failed_jobs").cloned().unwrap_or(json!([]));
        }
        Err(error) => {
            query_errors
                .push(json!({"query": "run_view", "run_id": run_id, "error": error.to_string()}));
        }
    }

    match queries.run_logs(&run_id, LogScope::Failed, bounds.log_max_bytes) {
        Ok(log) => {
            failure["log_excerpt"] = json!(log.text);
            failure["log_truncated"] = json!(log.truncated);
            failure["log_total_bytes"] = json!(log.total_bytes);
            failure["log_returned_bytes"] = json!(log.returned_bytes);
            failure["log_scope"] = json!("failed");
            failure["actual_checkout_shas"] = json!(log.checkout_commits);
            failure["checkout_evidence"] = json!(log.checkout_evidence);
            failure["checkout_evidence_scope"] = json!("failed");
            // `gh` can succeed with empty stdout when the run's logs are gone
            // (retention). That is not a captured excerpt; record it so the
            // filed task can say why the block is empty.
            if log.text.trim().is_empty() {
                query_errors.push(json!({
                    "query": "run_logs",
                    "run_id": run_id,
                    "error": "query returned no log text",
                }));
            }
        }
        Err(error) => {
            query_errors
                .push(json!({"query": "run_logs", "run_id": run_id, "error": error.to_string()}));
        }
    }

    // The checkout step normally succeeds, so it is absent from the
    // failed-step log. One full-log read per run, within a hard budget,
    // recovers the commit under test; past the budget we say so rather than
    // leaving the field silently empty.
    let needs_checkout = failure
        .get("actual_checkout_shas")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty);
    if !needs_checkout {
        return;
    }
    if *checkout_log_reads >= bounds.max_checkout_log_reads {
        failure["checkout_evidence_scope"] = json!("skipped_budget_exhausted");
        return;
    }
    *checkout_log_reads += 1;
    match queries.run_logs(&run_id, LogScope::All, bounds.log_max_bytes) {
        Ok(log) => {
            failure["actual_checkout_shas"] = json!(log.checkout_commits);
            failure["checkout_evidence"] = json!(log.checkout_evidence);
            failure["checkout_evidence_scope"] = json!("all");
        }
        Err(error) => {
            failure["checkout_evidence_scope"] = json!("unavailable");
            query_errors.push(
                json!({"query": "run_logs_all", "run_id": run_id, "error": error.to_string()}),
            );
        }
    }
}
