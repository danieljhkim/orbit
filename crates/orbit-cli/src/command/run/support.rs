use std::collections::BTreeSet;
use std::fmt::Write as _;

use orbit_core::{JobRun, JobRunState, JobRunStep, OrbitError, OrbitRuntime, find_workflow};
use serde_json::{Value, json};

const SHIP_AUTO_WORKFLOW: &str = "ship-auto";

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
    loop_count: u32,
) -> Result<Vec<WorkflowDispatchResult>, OrbitError> {
    let workflow = find_workflow(workflow_alias)
        .ok_or_else(|| OrbitError::InvalidInput(format!("unknown workflow '{workflow_alias}'")))?;
    if debug {
        return Err(OrbitError::InvalidInput(
            "`orbit run --debug` is not supported for persisted workflow runs; use `orbit job run <path>` for direct schemaVersion 2 job debugging.".to_string(),
        ));
    }

    let timeout_seconds = OrbitRuntime::normalize_pipeline_wait_timeout(None)?;
    let poll_interval_seconds = OrbitRuntime::normalize_pipeline_wait_poll_interval(None);

    let mut results = Vec::with_capacity(loop_count as usize);
    for _ in 0..loop_count {
        let invoke = runtime.submit_pipeline_run(workflow.job_id, input.clone(), None, None)?;
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
        let ship_auto = if workflow_alias == SHIP_AUTO_WORKFLOW {
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
    let error_code = run.error_code.clone().unwrap_or_else(|| "-".to_string());
    let error_message = single_line(run.error_message.as_deref().unwrap_or("-"));
    let mut first = format!(
        "workflow={};job_id={};run_id={};state={};attempt={}",
        run.workflow_alias, run.job_id, run.run_id, run.state, run.attempt
    );
    if let Some(summary) = &run.ship_auto {
        let reasons = if summary.exclusion_reasons.is_empty() {
            "-".to_string()
        } else {
            summary.exclusion_reasons.join(",")
        };
        let _ = write!(
            first,
            ";workflow_status={};candidate_tasks={};dispatched_bundles={};excluded_tasks={};exclusion_reasons={}",
            summary.status.as_str(),
            summary.candidate_task_count,
            summary.dispatched_bundle_count,
            summary.excluded_task_count,
            reasons
        );
    }
    let _ = write!(
        first,
        ";error_code={};error_message={}",
        single_line(&error_code),
        error_message
    );

    let mut lines = vec![first];
    if let Some(summary) = &run.ship_auto {
        for exclusion in &summary.exclusions {
            for conflict in &exclusion.conflicts {
                lines.push(format!(
                    "blocker workflow={};run_id={};task_id={};reason={};requested_selector={};holder_type={};holder_id={}",
                    run.workflow_alias,
                    run.run_id,
                    single_line(&exclusion.task_id),
                    single_line(&exclusion.reason),
                    single_line(&conflict.requested_selector),
                    single_line(&conflict.holder_type),
                    single_line(&conflict.holder_id)
                ));
            }
        }
        for child in summary
            .child_gate_runs
            .iter()
            .filter(|child| child.is_waiting())
        {
            lines.push(format!(
                "gate_child workflow={};run_id={};child_run_id={};wait_status={};current_status={};activity={}",
                run.workflow_alias,
                run.run_id,
                single_line(&child.run_id),
                single_line(&child.wait_status),
                single_line(&child.current_status),
                single_line(child.activity.as_deref().unwrap_or("-"))
            ));
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
    use super::*;

    fn ship_auto_run(summary: ShipAutoDispatchSummary) -> WorkflowDispatchResult {
        WorkflowDispatchResult {
            workflow_alias: SHIP_AUTO_WORKFLOW,
            job_id: "task_auto_pipeline".to_string(),
            run_id: "jrun-parent".to_string(),
            state: "succeeded".to_string(),
            attempt: 1,
            error_code: None,
            error_message: None,
            ship_auto: Some(summary),
        }
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
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("workflow_status=empty_backlog"));
        assert!(lines[0].contains("candidate_tasks=0"));
        assert!(lines[0].contains("excluded_tasks=0"));

        let value = workflow_dispatch_result_to_json(&run);
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
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("workflow_status=gated_noop"));
        assert!(lines[0].contains("dispatched_bundles=0"));
        assert!(lines[0].contains("excluded_tasks=1"));
        assert!(lines[1].contains("blocker workflow=ship-auto"));
        assert!(lines[1].contains("task_id=T20260430-blocked"));
        assert!(lines[1].contains("requested_selector=file:crates/foo/src/lib.rs"));
        assert!(lines[1].contains("holder_type=task"));
        assert!(lines[1].contains("holder_id=T20260430-locking"));

        let value = workflow_dispatch_result_to_json(&run);
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
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("workflow_status=gate_waiting"));
        assert!(lines[0].contains("dispatched_bundles=1"));
        assert!(lines[1].contains("gate_child workflow=ship-auto"));
        assert!(lines[1].contains("child_run_id=jrun-child"));
        assert!(lines[1].contains("wait_status=timeout"));
        assert!(lines[1].contains("current_status=running"));
        assert!(lines[1].contains("activity=reserve"));

        let value = workflow_dispatch_result_to_json(&run);
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
