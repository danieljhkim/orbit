use std::collections::BTreeSet;
use std::fmt::Write as _;

use orbit_core::{JobRun, JobRunState, JobRunStep, OrbitError, OrbitRuntime, find_workflow};
use serde_json::{Value, json};

const TASK_AUTO_PIPELINE_JOB: &str = "task_auto_pipeline";

#[derive(Clone)]
pub(crate) struct WorkflowDispatchResult {
    pub workflow_alias: &'static str,
    pub job_id: String,
    pub run_id: String,
    pub state: String,
    pub attempt: u32,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub ship_auto: Option<ShipAutoDispatchSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShipAutoDispatchSummary {
    pub status: ShipAutoStatus,
    pub candidate_task_count: usize,
    pub dispatched_bundle_count: usize,
    pub excluded_task_count: usize,
    pub exclusion_reasons: Vec<String>,
    pub exclusions: Vec<ShipAutoExclusion>,
    pub child_gate_runs: Vec<ShipAutoGateRun>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShipAutoStatus {
    EmptyBacklog,
    GatedNoop,
    GateWaiting,
    GateFailed,
    Completed,
}

impl ShipAutoStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::EmptyBacklog => "empty_backlog",
            Self::GatedNoop => "gated_noop",
            Self::GateWaiting => "gate_waiting",
            Self::GateFailed => "gate_failed",
            Self::Completed => "completed",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::EmptyBacklog => "Empty backlog",
            Self::GatedNoop => "Gated no-op",
            Self::GateWaiting => "Gate waiting",
            Self::GateFailed => "Gate failed",
            Self::Completed => "Completed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShipAutoExclusion {
    pub task_id: String,
    pub reason: String,
    pub conflicts: Vec<ShipAutoConflict>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShipAutoConflict {
    pub requested_selector: String,
    pub holder_type: String,
    pub holder_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShipAutoGateRun {
    pub run_id: String,
    pub wait_status: String,
    pub current_status: String,
    pub activity: Option<String>,
}

pub(crate) fn dispatch_workflow(
    runtime: &OrbitRuntime,
    workflow_alias: &'static str,
    input: &Value,
    debug: bool,
    wait_for_completion: bool,
    loop_count: u32,
) -> Result<Vec<WorkflowDispatchResult>, OrbitError> {
    let workflow = find_workflow(workflow_alias)
        .ok_or_else(|| OrbitError::InvalidInput(format!("unknown workflow '{workflow_alias}'")))?;
    if debug {
        return Err(OrbitError::InvalidInput(
            "`orbit run --debug` is not supported for persisted workflow runs; use `orbit job run <path>` for direct schemaVersion 2 job debugging.".to_string(),
        ));
    }

    let mut results = Vec::with_capacity(loop_count as usize);
    for _ in 0..loop_count {
        let invoke = runtime.submit_pipeline_run(workflow.job_id, input.clone(), None, None)?;
        if !wait_for_completion {
            let state = if invoke.queued { "queued" } else { "submitted" };
            results.push(WorkflowDispatchResult {
                workflow_alias,
                job_id: invoke.job_name,
                run_id: invoke.run_id,
                state: state.to_string(),
                attempt: 1,
                error_code: None,
                error_message: None,
                ship_auto: None,
            });
            continue;
        }

        let timeout_seconds = OrbitRuntime::normalize_pipeline_wait_timeout(None)?;
        let poll_interval_seconds = OrbitRuntime::normalize_pipeline_wait_poll_interval(None);
        let wait = runtime.wait_pipeline_runs(
            std::slice::from_ref(&invoke.run_id),
            timeout_seconds,
            poll_interval_seconds,
            None,
        )?;
        let run = runtime.show_job_run(&invoke.run_id)?;
        let run_details = runtime
            .job_history(workflow.job_id)?
            .into_iter()
            .find(|entry| entry.run_id == run.run_id);
        let wait_entry = wait
            .results
            .into_iter()
            .find(|entry| entry.run_id == run.run_id);
        let pipeline = wait_entry
            .as_ref()
            .and_then(|entry| entry.pipeline.clone())
            .or_else(|| {
                runtime
                    .read_run_state(&run.run_id)
                    .ok()
                    .flatten()
                    .map(|state| state.pipeline)
            });
        let ship_auto = if workflow.job_id == TASK_AUTO_PIPELINE_JOB {
            Some(derive_ship_auto_summary(runtime, pipeline.as_ref()))
        } else {
            None
        };
        results.push(WorkflowDispatchResult {
            workflow_alias,
            job_id: run.job_id,
            run_id: run.run_id,
            state: wait_entry
                .as_ref()
                .map(|entry| entry.status.clone())
                .unwrap_or_else(|| run.state.to_string()),
            attempt: run.attempt,
            error_code: run_details
                .as_ref()
                .and_then(summary_step)
                .and_then(|step| step.error_code.clone()),
            error_message: wait_entry.and_then(|entry| entry.error).or_else(|| {
                run_details
                    .as_ref()
                    .and_then(summary_step)
                    .and_then(|step| step.error_message.clone())
            }),
            ship_auto,
        });
    }

    Ok(results)
}

pub(crate) fn print_workflow_dispatch_results(
    workflow_alias: &'static str,
    runs: &[WorkflowDispatchResult],
    json_output: bool,
) -> Result<(), OrbitError> {
    if json_output {
        if runs.len() == 1 {
            return crate::output::json::print_pretty(&workflow_dispatch_result_to_json(&runs[0]));
        }
        return crate::output::json::print_pretty(&json!({
            "workflow": workflow_alias,
            "runs": runs
                .iter()
                .map(workflow_dispatch_result_to_json)
                .collect::<Vec<_>>(),
        }));
    }

    for run in runs {
        for line in workflow_dispatch_result_lines(run) {
            println!("{line}");
        }
    }
    Ok(())
}

fn summary_step(run: &JobRun) -> Option<&JobRunStep> {
    run.steps
        .iter()
        .rev()
        .find(|step| step.error_code.is_some() || step.error_message.is_some())
        .or_else(|| {
            run.steps.iter().rev().find(|step| {
                matches!(
                    step.state,
                    JobRunState::Failed | JobRunState::Timeout | JobRunState::Cancelled
                )
            })
        })
        .or_else(|| {
            run.steps
                .iter()
                .rev()
                .find(|step| step.state != JobRunState::Skipped)
        })
        .or_else(|| run.steps.last())
}

fn workflow_dispatch_result_to_json(run: &WorkflowDispatchResult) -> Value {
    let mut value = json!({
        "workflow": run.workflow_alias,
        "job_id": run.job_id,
        "run_id": run.run_id,
        "state": run.state,
        "attempt": run.attempt,
        "error_code": run.error_code,
        "error_message": run.error_message,
    });
    if let Some(summary) = &run.ship_auto
        && let Some(object) = value.as_object_mut()
    {
        object.insert(
            "workflow_status".to_string(),
            Value::String(summary.status.as_str().to_string()),
        );
        object.insert(
            "dispatched_bundle_count".to_string(),
            json!(summary.dispatched_bundle_count),
        );
        object.insert(
            "excluded_task_count".to_string(),
            json!(summary.excluded_task_count),
        );
        object.insert(
            "exclusion_reasons".to_string(),
            json!(summary.exclusion_reasons),
        );
        object.insert(
            "conflict_holders".to_string(),
            summary.conflict_holders_json(),
        );
        object.insert("ship_auto".to_string(), summary.to_json());
    }
    value
}

fn workflow_dispatch_result_lines(run: &WorkflowDispatchResult) -> Vec<String> {
    if let Some(summary) = &run.ship_auto {
        return ship_auto_dispatch_result_lines(run, summary);
    }

    if matches!(run.state.as_str(), "submitted" | "queued")
        && run.error_code.is_none()
        && run.error_message.is_none()
    {
        return vec![
            format!("Workflow: {}", run.workflow_alias),
            format!("Job ID: {}", run.job_id),
            format!("Run ID: {}", run.run_id),
            format!("State: {}", run.state),
            format!(
                "Inspect: orbit run history -j {} | orbit run show {}",
                run.job_id, run.run_id
            ),
        ];
    }

    let error_code = run.error_code.clone().unwrap_or_else(|| "-".to_string());
    let error_message = single_line(run.error_message.as_deref().unwrap_or("-"));
    let mut first = format!(
        "workflow={};job_id={};run_id={};state={};attempt={}",
        run.workflow_alias, run.job_id, run.run_id, run.state, run.attempt
    );
    let _ = write!(
        first,
        ";error_code={};error_message={}",
        single_line(&error_code),
        error_message
    );

    vec![first]
}

fn ship_auto_dispatch_result_lines(
    run: &WorkflowDispatchResult,
    summary: &ShipAutoDispatchSummary,
) -> Vec<String> {
    let mut lines = vec![
        format!("Workflow: {}", run.workflow_alias),
        format!("Job ID: {}", run.job_id),
        format!("Run ID: {}", run.run_id),
        format!("Parent state: {}", run.state),
        format!("Attempt: {}", run.attempt),
        format!("Status: {}", summary.status.label()),
        format!("Candidate tasks: {}", summary.candidate_task_count),
        format!("Dispatched bundles: {}", summary.dispatched_bundle_count),
        format!("Excluded tasks: {}", summary.excluded_task_count),
    ];

    if !summary.exclusion_reasons.is_empty() {
        lines.push(format!(
            "Exclusion reasons: {}",
            summary
                .exclusion_reasons
                .iter()
                .map(|reason| display_token(reason))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(error_code) = &run.error_code {
        lines.push(format!("Error code: {}", single_line(error_code)));
    }
    if let Some(error_message) = &run.error_message {
        lines.push(format!("Error message: {}", single_line(error_message)));
    }

    let has_blockers = summary
        .exclusions
        .iter()
        .any(|exclusion| !exclusion.conflicts.is_empty());
    if has_blockers {
        lines.push("Blockers:".to_string());
        for exclusion in &summary.exclusions {
            for conflict in &exclusion.conflicts {
                lines.push(format!("  - Task: {}", single_line(&exclusion.task_id)));
                lines.push(format!("    Reason: {}", display_token(&exclusion.reason)));
                lines.push(format!(
                    "    Requested selector: {}",
                    single_line(&conflict.requested_selector)
                ));
                lines.push(format!(
                    "    Holder type: {}",
                    single_line(&conflict.holder_type)
                ));
                lines.push(format!(
                    "    Holder ID: {}",
                    single_line(&conflict.holder_id)
                ));
            }
        }
    }

    let notable_child_runs = summary
        .child_gate_runs
        .iter()
        .filter(|child| match summary.status {
            ShipAutoStatus::GateWaiting => child.is_waiting(),
            ShipAutoStatus::GateFailed => child.is_failed(),
            _ => false,
        })
        .collect::<Vec<_>>();
    if !notable_child_runs.is_empty() {
        lines.push("Child gate runs:".to_string());
        for child in notable_child_runs {
            lines.push(format!("  - Child run ID: {}", single_line(&child.run_id)));
            lines.push(format!(
                "    Wait status: {}",
                single_line(&child.wait_status)
            ));
            lines.push(format!(
                "    Current status: {}",
                single_line(&child.current_status)
            ));
            if let Some(activity) = &child.activity {
                lines.push(format!("    Activity: {}", single_line(activity)));
            }
        }
    }

    lines
}

fn derive_ship_auto_summary(
    runtime: &OrbitRuntime,
    pipeline: Option<&Value>,
) -> ShipAutoDispatchSummary {
    let child_gate_runs = pipeline
        .map(|pipeline| child_gate_runs_from_pipeline(runtime, pipeline))
        .unwrap_or_default();
    summarize_ship_auto_pipeline(pipeline, child_gate_runs)
}

fn summarize_ship_auto_pipeline(
    pipeline: Option<&Value>,
    child_gate_runs: Vec<ShipAutoGateRun>,
) -> ShipAutoDispatchSummary {
    let list_backlog = pipeline.and_then(|value| value.get("list_backlog"));
    let candidate_task_count = value_count(list_backlog, "task_count", "tasks");
    let exclusions = parse_exclusions(list_backlog.and_then(|value| value.get("excluded")));
    let excluded_task_count = exclusions.len();
    let exclusion_reasons = exclusion_reasons(&exclusions);
    let dispatched_bundle_count = pipeline
        .and_then(|value| value.get("validate_bundles"))
        .map(|value| value_count(Some(value), "bundle_count", "bundles"))
        .or_else(|| {
            pipeline
                .and_then(|value| value.get("dispatch"))
                .and_then(Value::as_array)
                .map(Vec::len)
        })
        .unwrap_or(0);
    let status = if child_gate_runs.iter().any(ShipAutoGateRun::is_waiting) {
        ShipAutoStatus::GateWaiting
    } else if child_gate_runs.iter().any(ShipAutoGateRun::is_failed) {
        ShipAutoStatus::GateFailed
    } else if dispatched_bundle_count == 0 && excluded_task_count > 0 {
        ShipAutoStatus::GatedNoop
    } else if dispatched_bundle_count == 0 && candidate_task_count == 0 && excluded_task_count == 0
    {
        ShipAutoStatus::EmptyBacklog
    } else {
        ShipAutoStatus::Completed
    };

    ShipAutoDispatchSummary {
        status,
        candidate_task_count,
        dispatched_bundle_count,
        excluded_task_count,
        exclusion_reasons,
        exclusions,
        child_gate_runs,
    }
}

fn child_gate_runs_from_pipeline(runtime: &OrbitRuntime, pipeline: &Value) -> Vec<ShipAutoGateRun> {
    let Some(gate_results) = pipeline
        .get("gate_results")
        .or_else(|| pipeline.get("dispatch"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    gate_results
        .iter()
        .filter_map(|entry| {
            let run_id = entry.get("run_id").and_then(Value::as_str)?;
            let wait_status = entry
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            Some((run_id.to_string(), wait_status))
        })
        .map(|(run_id, wait_status)| {
            let (current_status, activity) = runtime
                .show_job_run(&run_id)
                .map(|run| (run.state.to_string(), gate_activity(&run)))
                .unwrap_or_else(|_| (wait_status.clone(), None));
            ShipAutoGateRun {
                run_id,
                wait_status,
                current_status,
                activity,
            }
        })
        .collect()
}

fn value_count(value: Option<&Value>, count_key: &str, array_key: &str) -> usize {
    value
        .and_then(|value| value.get(count_key))
        .and_then(Value::as_u64)
        .map(|count| count as usize)
        .or_else(|| {
            value
                .and_then(|value| value.get(array_key))
                .and_then(Value::as_array)
                .map(Vec::len)
        })
        .unwrap_or(0)
}

fn parse_exclusions(value: Option<&Value>) -> Vec<ShipAutoExclusion> {
    value
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let task_id = entry.get("id").and_then(Value::as_str)?.to_string();
                    let reason = entry
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string();
                    let conflicts = entry
                        .get("conflicts")
                        .and_then(Value::as_array)
                        .map(|items| items.iter().filter_map(parse_conflict).collect())
                        .unwrap_or_default();
                    Some(ShipAutoExclusion {
                        task_id,
                        reason,
                        conflicts,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_conflict(value: &Value) -> Option<ShipAutoConflict> {
    let requested_selector = value
        .get("requested_selector")
        .or_else(|| value.get("requested_file"))
        .or_else(|| value.get("file"))
        .and_then(Value::as_str)?
        .to_string();
    let (holder_type, holder_id) =
        if let Some(locking_task_id) = value.get("locking_task_id").and_then(Value::as_str) {
            ("task".to_string(), locking_task_id.to_string())
        } else if let Some(reservation_id) = value.get("reservation_id").and_then(Value::as_str) {
            ("reservation".to_string(), reservation_id.to_string())
        } else {
            let holder_type = value
                .get("held_by")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let holder_id = value.get("held_by_id").and_then(Value::as_str)?.to_string();
            (holder_type, holder_id)
        };

    Some(ShipAutoConflict {
        requested_selector,
        holder_type,
        holder_id,
    })
}

fn exclusion_reasons(exclusions: &[ShipAutoExclusion]) -> Vec<String> {
    exclusions
        .iter()
        .map(|entry| entry.reason.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn gate_activity(run: &JobRun) -> Option<String> {
    run.steps
        .iter()
        .filter(|step| step.state != JobRunState::Skipped)
        .max_by_key(|step| step.step_index)
        .map(|step| step.target_id.to_string())
        .or_else(|| match run.state {
            JobRunState::Pending => Some("queued".to_string()),
            JobRunState::Running => Some("running".to_string()),
            _ => None,
        })
}

fn single_line(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}

fn display_token(value: &str) -> String {
    single_line(value).replace('_', " ")
}

impl ShipAutoGateRun {
    fn is_waiting(&self) -> bool {
        matches!(self.wait_status.as_str(), "pending" | "running")
            || matches!(
                self.current_status.as_str(),
                "pending" | "running" | "retrying"
            )
            || (self.wait_status == "timeout" && !is_terminal_gate_status(&self.current_status))
    }

    fn is_failed(&self) -> bool {
        matches!(self.wait_status.as_str(), "failed" | "cancelled")
            || matches!(
                self.current_status.as_str(),
                "failed" | "cancelled" | "timeout"
            )
    }

    fn to_json(&self) -> Value {
        json!({
            "run_id": self.run_id,
            "wait_status": self.wait_status,
            "current_status": self.current_status,
            "activity": self.activity,
        })
    }
}

fn is_terminal_gate_status(status: &str) -> bool {
    matches!(
        status,
        "success" | "succeeded" | "failed" | "cancelled" | "timeout"
    )
}

impl ShipAutoDispatchSummary {
    fn to_json(&self) -> Value {
        json!({
            "status": self.status.as_str(),
            "candidate_task_count": self.candidate_task_count,
            "dispatched_bundle_count": self.dispatched_bundle_count,
            "excluded_task_count": self.excluded_task_count,
            "exclusion_reasons": self.exclusion_reasons,
            "conflict_holders": self.conflict_holders_json(),
            "exclusions": self
                .exclusions
                .iter()
                .map(ShipAutoExclusion::to_json)
                .collect::<Vec<_>>(),
            "child_gate_runs": self
                .child_gate_runs
                .iter()
                .map(ShipAutoGateRun::to_json)
                .collect::<Vec<_>>(),
        })
    }

    fn conflict_holders_json(&self) -> Value {
        let holders = self
            .exclusions
            .iter()
            .flat_map(|entry| entry.conflicts.iter())
            .map(|conflict| {
                json!({
                    "type": conflict.holder_type,
                    "id": conflict.holder_id,
                })
            })
            .collect::<Vec<_>>();
        Value::Array(holders)
    }
}

impl ShipAutoExclusion {
    fn to_json(&self) -> Value {
        json!({
            "task_id": self.task_id,
            "reason": self.reason,
            "conflicts": self
                .conflicts
                .iter()
                .map(ShipAutoConflict::to_json)
                .collect::<Vec<_>>(),
        })
    }
}

impl ShipAutoConflict {
    fn to_json(&self) -> Value {
        json!({
            "requested_selector": self.requested_selector,
            "holder_type": self.holder_type,
            "holder_id": self.holder_id,
            "locking_task_id": (self.holder_type == "task").then(|| self.holder_id.clone()),
            "reservation_id": (self.holder_type == "reservation").then(|| self.holder_id.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    const SHIP_WORKFLOW: &str = "ship";

    #[test]
    fn async_ship_dispatch_returns_run_identity_without_waiting() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let jobs_dir = runtime.data_root().join("resources/jobs");
        std::fs::create_dir_all(&jobs_dir).expect("create jobs dir");
        std::fs::write(
            jobs_dir.join("task_auto_pipeline.yaml"),
            r#"schemaVersion: 2
kind: Job
metadata:
  name: task_auto_pipeline
spec:
  state: enabled
  kind: workflow
  steps:
    - id: marker
      spec:
        type: deterministic
        action: sleep
        config:
          seconds: 0
"#,
        )
        .expect("write task_auto_pipeline fixture");
        let started = Instant::now();
        let runs = dispatch_workflow(
            &runtime,
            SHIP_WORKFLOW,
            &json!({
                "mode": "pr",
                "base_branch": "main",
            }),
            false,
            false,
            1,
        )
        .expect("dispatch workflow");

        assert!(
            started.elapsed() < Duration::from_secs(1),
            "dispatch waited too long"
        );
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].workflow_alias, SHIP_WORKFLOW);
        assert_eq!(runs[0].job_id, TASK_AUTO_PIPELINE_JOB);
        assert!(matches!(runs[0].state.as_str(), "submitted" | "queued"));
        assert!(runs[0].ship_auto.is_none());
    }

    fn ship_auto_run(summary: ShipAutoDispatchSummary) -> WorkflowDispatchResult {
        WorkflowDispatchResult {
            workflow_alias: SHIP_WORKFLOW,
            job_id: TASK_AUTO_PIPELINE_JOB.to_string(),
            run_id: "jrun-parent".to_string(),
            state: "succeeded".to_string(),
            attempt: 1,
            error_code: None,
            error_message: None,
            ship_auto: Some(summary),
        }
    }

    fn assert_ship_auto_json_contract(value: &Value) {
        for key in [
            "workflow",
            "job_id",
            "run_id",
            "state",
            "attempt",
            "workflow_status",
            "dispatched_bundle_count",
            "excluded_task_count",
            "exclusion_reasons",
            "conflict_holders",
            "ship_auto",
        ] {
            assert!(value.get(key).is_some(), "missing json key {key}");
        }
        assert_eq!(value["workflow"], json!("ship"));
        assert_eq!(value["job_id"], json!("task_auto_pipeline"));
        assert_eq!(value["run_id"], json!("jrun-parent"));
        assert_eq!(value["state"], json!("succeeded"));
        assert_eq!(value["attempt"], json!(1));
    }

    #[test]
    fn async_dispatch_lines_point_to_history_and_show() {
        let run = WorkflowDispatchResult {
            workflow_alias: SHIP_WORKFLOW,
            job_id: TASK_AUTO_PIPELINE_JOB.to_string(),
            run_id: "jrun-submitted".to_string(),
            state: "submitted".to_string(),
            attempt: 1,
            error_code: None,
            error_message: None,
            ship_auto: None,
        };

        assert_eq!(
            workflow_dispatch_result_lines(&run),
            vec![
                "Workflow: ship",
                "Job ID: task_auto_pipeline",
                "Run ID: jrun-submitted",
                "State: submitted",
                "Inspect: orbit run history -j task_auto_pipeline | orbit run show jrun-submitted",
            ]
        );

        let value = workflow_dispatch_result_to_json(&run);
        assert_eq!(value["workflow"], json!("ship"));
        assert_eq!(value["job_id"], json!("task_auto_pipeline"));
        assert_eq!(value["state"], json!("submitted"));
        assert_eq!(value["error_code"], Value::Null);
        assert_eq!(value["error_message"], Value::Null);
        assert!(value.get("ship_auto").is_none());
    }

    #[test]
    fn ship_auto_summary_reports_true_empty_backlog() {
        let pipeline = json!({
            "list_backlog": {
                "task_count": 0,
                "task_ids": [],
                "tasks": [],
                "bundles": [],
                "excluded": []
            },
            "validate_bundles": {
                "bundles": [],
                "bundle_count": 0
            },
            "gate_results": []
        });

        let summary = summarize_ship_auto_pipeline(Some(&pipeline), Vec::new());

        assert_eq!(summary.status, ShipAutoStatus::EmptyBacklog);
        assert_eq!(summary.candidate_task_count, 0);
        assert_eq!(summary.dispatched_bundle_count, 0);
        assert_eq!(summary.excluded_task_count, 0);
        assert!(summary.exclusions.is_empty());

        let run = ship_auto_run(summary);
        let lines = workflow_dispatch_result_lines(&run);
        assert_eq!(
            lines,
            vec![
                "Workflow: ship",
                "Job ID: task_auto_pipeline",
                "Run ID: jrun-parent",
                "Parent state: succeeded",
                "Attempt: 1",
                "Status: Empty backlog",
                "Candidate tasks: 0",
                "Dispatched bundles: 0",
                "Excluded tasks: 0",
            ]
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("error_code=-") || line.contains("error_message=-"))
        );

        let value = workflow_dispatch_result_to_json(&run);
        assert_ship_auto_json_contract(&value);
        assert_eq!(value["workflow_status"], json!("empty_backlog"));
        assert_eq!(value["dispatched_bundle_count"], json!(0));
        assert_eq!(value["excluded_task_count"], json!(0));
        assert_eq!(value["ship_auto"]["conflict_holders"], json!([]));
    }

    #[test]
    fn ship_auto_summary_reports_gated_noop_context_lock_blocker() {
        let pipeline = json!({
            "list_backlog": {
                "task_count": 0,
                "task_ids": [],
                "tasks": [],
                "bundles": [],
                "excluded": [{
                    "id": "T20260430-blocked",
                    "reason": "context_lock_conflict",
                    "conflicts": [{
                        "requested_file": "file:crates/foo/src/lib.rs",
                        "locking_task_id": "T20260430-locking"
                    }]
                }]
            },
            "validate_bundles": {
                "bundles": [],
                "bundle_count": 0
            },
            "gate_results": []
        });

        let summary = summarize_ship_auto_pipeline(Some(&pipeline), Vec::new());

        assert_eq!(summary.status, ShipAutoStatus::GatedNoop);
        assert_eq!(summary.dispatched_bundle_count, 0);
        assert_eq!(summary.excluded_task_count, 1);
        assert_eq!(
            summary.exclusion_reasons,
            vec!["context_lock_conflict".to_string()]
        );

        let run = ship_auto_run(summary);
        let lines = workflow_dispatch_result_lines(&run);
        assert!(lines.iter().any(|line| line == "Status: Gated no-op"));
        assert!(lines.iter().any(|line| line == "Dispatched bundles: 0"));
        assert!(lines.iter().any(|line| line == "Excluded tasks: 1"));
        assert!(
            lines
                .iter()
                .any(|line| line == "Exclusion reasons: context lock conflict")
        );
        assert!(lines.iter().any(|line| line == "Blockers:"));
        assert!(
            lines
                .iter()
                .any(|line| line == "  - Task: T20260430-blocked")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "    Requested selector: file:crates/foo/src/lib.rs")
        );
        assert!(lines.iter().any(|line| line == "    Holder type: task"));
        assert!(
            lines
                .iter()
                .any(|line| line == "    Holder ID: T20260430-locking")
        );
        assert!(!lines.iter().any(|line| line.contains("blocker workflow=")));

        let value = workflow_dispatch_result_to_json(&run);
        assert_ship_auto_json_contract(&value);
        assert_eq!(value["workflow_status"], json!("gated_noop"));
        assert_eq!(value["dispatched_bundle_count"], json!(0));
        assert_eq!(value["excluded_task_count"], json!(1));
        assert_eq!(value["exclusion_reasons"], json!(["context_lock_conflict"]));
        assert_eq!(
            value["conflict_holders"],
            json!([{ "type": "task", "id": "T20260430-locking" }])
        );
        assert_eq!(
            value["ship_auto"]["exclusions"][0]["conflicts"][0]["locking_task_id"],
            json!("T20260430-locking")
        );
    }

    #[test]
    fn ship_auto_summary_reports_waiting_gate_children() {
        let pipeline = json!({
            "list_backlog": {
                "task_count": 1,
                "task_ids": ["T20260430-ready"],
                "tasks": [{ "id": "T20260430-ready" }],
                "bundles": [["T20260430-ready"]],
                "excluded": []
            },
            "validate_bundles": {
                "bundles": [["T20260430-ready"]],
                "bundle_count": 1
            },
            "gate_results": [{
                "run_id": "jrun-child",
                "status": "timeout"
            }]
        });
        let child_runs = vec![ShipAutoGateRun {
            run_id: "jrun-child".to_string(),
            wait_status: "timeout".to_string(),
            current_status: "running".to_string(),
            activity: Some("reserve".to_string()),
        }];

        let summary = summarize_ship_auto_pipeline(Some(&pipeline), child_runs);

        assert_eq!(summary.status, ShipAutoStatus::GateWaiting);
        assert_eq!(summary.candidate_task_count, 1);
        assert_eq!(summary.dispatched_bundle_count, 1);
        assert_eq!(summary.excluded_task_count, 0);

        let run = ship_auto_run(summary);
        let lines = workflow_dispatch_result_lines(&run);
        assert!(lines.iter().any(|line| line == "Status: Gate waiting"));
        assert!(lines.iter().any(|line| line == "Dispatched bundles: 1"));
        assert!(lines.iter().any(|line| line == "Child gate runs:"));
        assert!(
            lines
                .iter()
                .any(|line| line == "  - Child run ID: jrun-child")
        );
        assert!(lines.iter().any(|line| line == "    Wait status: timeout"));
        assert!(
            lines
                .iter()
                .any(|line| line == "    Current status: running")
        );
        assert!(lines.iter().any(|line| line == "    Activity: reserve"));
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("gate_child workflow="))
        );

        let value = workflow_dispatch_result_to_json(&run);
        assert_ship_auto_json_contract(&value);
        assert_eq!(value["workflow_status"], json!("gate_waiting"));
        assert_eq!(
            value["ship_auto"]["child_gate_runs"][0]["run_id"],
            json!("jrun-child")
        );
        assert_eq!(
            value["ship_auto"]["child_gate_runs"][0]["current_status"],
            json!("running")
        );
    }
}
