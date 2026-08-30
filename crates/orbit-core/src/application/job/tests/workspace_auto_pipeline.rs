use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use orbit_engine::{DispatchError, ResolvedCliExecutor, RuntimeHost};
use orbit_tools::{FsAuditLogger, ToolContext};
use orbit_types::task::TaskStatus;
use orbit_types::telemetry::AuditEventStatus;
use orbit_types::workflow::{ChildDispatch, ChildDispatchPhase, PipelineState};
use serde_json::{Value, json};

use crate::OrbitRuntime;

use super::exec::{seed_default_catalogs, seed_gate_task, test_runtime, try_execute_named_job};

#[derive(Clone, Copy)]
enum WorkspaceAutoScenario {
    MixedThenLater,
    TerminalNonSuccesses,
    DispatchFailure,
    MalformedResult,
    ClassifierFailure,
}

struct ScriptedWorkspaceAutoHost<'a> {
    runtime: &'a OrbitRuntime,
    scenario: WorkspaceAutoScenario,
    failed_task_id: String,
    classify_calls: AtomicUsize,
    window_calls: AtomicUsize,
    calls: Mutex<Vec<(String, Value)>>,
    dispatch_state_lock: Mutex<()>,
}

impl<'a> ScriptedWorkspaceAutoHost<'a> {
    fn new(
        runtime: &'a OrbitRuntime,
        scenario: WorkspaceAutoScenario,
        failed_task_id: impl Into<String>,
    ) -> Self {
        Self {
            runtime,
            scenario,
            failed_task_id: failed_task_id.into(),
            classify_calls: AtomicUsize::new(0),
            window_calls: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
            dispatch_state_lock: Mutex::new(()),
        }
    }

    fn inputs_for(&self, action: &str) -> Vec<Value> {
        self.calls
            .lock()
            .expect("call log")
            .iter()
            .filter(|(recorded, _)| recorded == action)
            .map(|(_, input)| input.clone())
            .collect()
    }

    fn classification(&self) -> Value {
        let iteration = self.classify_calls.fetch_add(1, Ordering::SeqCst);
        let dispatches = match self.scenario {
            WorkspaceAutoScenario::MixedThenLater if iteration == 0 => vec![
                json!({"crew": "sol", "task_ids": ["ORB-SUCCEEDED"]}),
                json!({"crew": "terra", "task_ids": [self.failed_task_id]}),
            ],
            WorkspaceAutoScenario::MixedThenLater => {
                vec![json!({"crew": "sol", "task_ids": ["ORB-LATER"]})]
            }
            WorkspaceAutoScenario::TerminalNonSuccesses => vec![
                json!({"crew": "sol", "task_ids": ["ORB-FAILED"]}),
                json!({"crew": "terra", "task_ids": ["ORB-CANCELLED"]}),
                json!({"crew": "luna", "task_ids": ["ORB-INTERRUPTED"]}),
                json!({"crew": "opus", "task_ids": ["ORB-TIMEOUT"]}),
            ],
            WorkspaceAutoScenario::DispatchFailure => {
                vec![json!({"crew": "sol", "task_ids": ["ORB-DISPATCH-BROKEN"]})]
            }
            WorkspaceAutoScenario::MalformedResult => {
                vec![json!({"crew": "sol", "task_ids": ["ORB-MALFORMED"]})]
            }
            WorkspaceAutoScenario::ClassifierFailure => {
                unreachable!("classifier failure returns before building a classification")
            }
        };
        let task_ids = dispatches
            .iter()
            .flat_map(|dispatch| {
                dispatch["task_ids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .cloned()
            })
            .collect::<Vec<_>>();
        json!({
            "loose_task_ids": task_ids,
            "loose_task_dispatches": dispatches,
            "has_leaves": true,
            "epic_task_id": null,
            "has_epic": false,
            "empty": false,
            "active_epic_run_id": null,
            "active_epic_task_id": null,
        })
    }

    fn record_terminal_child(&self, input: &Value, result: &Value) {
        let Some(parent_run_id) = input.get("run_id").and_then(Value::as_str) else {
            return;
        };
        let Some(child_run_id) = result.get("run_id").and_then(Value::as_str) else {
            return;
        };
        let _guard = self
            .dispatch_state_lock
            .lock()
            .expect("dispatch state lock");
        let Some(mut state) = self
            .runtime
            .read_run_state(parent_run_id)
            .expect("read scripted parent state")
        else {
            return;
        };
        let status = result["status"].as_str().expect("scripted terminal status");
        let error = result
            .get("error")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let mut dispatch = ChildDispatch::submitted(
            child_run_id.to_string(),
            "task_auto_pipeline".to_string(),
            "invoke_and_wait".to_string(),
            true,
            false,
            Utc::now(),
        )
        .with_parent_step_id(
            input
                .get("step_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        );
        dispatch.phase = ChildDispatchPhase::Terminal;
        dispatch.child_status = Some(status.to_string());
        dispatch.error = error;
        state.record_child_dispatch(dispatch);
        self.runtime
            .write_run_state(parent_run_id, &state)
            .expect("write scripted parent state");
    }

    fn invoke_result(&self, input: &Value) -> Result<Value, DispatchError> {
        let task_id = input["run_input"]["task_ids"][0]
            .as_str()
            .expect("scripted task id");
        let result = if task_id == "ORB-DISPATCH-BROKEN" {
            return Err(DispatchError::DeterministicActionFailed {
                action: "invoke_and_wait".to_string(),
                message: "fixture dispatch failed before durable child creation".to_string(),
            });
        } else if task_id == "ORB-MALFORMED" {
            json!({
                "status": "failed",
                "error": "fixture omitted durable child linkage",
            })
        } else if task_id == self.failed_task_id {
            json!({
                "run_id": "jrun-scripted-failed-leaf",
                "status": "failed",
                "error": "fixture leaf implementation failed",
            })
        } else if task_id == "ORB-FAILED" {
            json!({
                "run_id": "jrun-scripted-failed",
                "status": "failed",
                "error": "fixture failed",
            })
        } else if task_id == "ORB-CANCELLED" {
            json!({
                "run_id": "jrun-scripted-cancelled",
                "status": "cancelled",
                "error": "fixture cancelled",
            })
        } else if task_id == "ORB-INTERRUPTED" {
            json!({
                "run_id": "jrun-scripted-interrupted",
                "status": "interrupted",
                "error": "fixture interrupted",
            })
        } else if task_id == "ORB-TIMEOUT" {
            json!({
                "run_id": "jrun-scripted-timeout",
                "status": "timeout",
                "error": "fixture timeout",
            })
        } else if task_id == "ORB-SUCCEEDED" {
            json!({
                "run_id": "jrun-scripted-succeeded-leaf",
                "status": "succeeded",
            })
        } else {
            json!({
                "run_id": "jrun-scripted-later-leaf",
                "status": "succeeded",
            })
        };
        self.record_terminal_child(input, &result);
        Ok(result)
    }
}

impl RuntimeHost for ScriptedWorkspaceAutoHost<'_> {
    fn run_deterministic(
        &self,
        action: &str,
        config: &Value,
        input: &Value,
        tool_context: ToolContext,
    ) -> Result<Value, DispatchError> {
        self.calls
            .lock()
            .expect("call log")
            .push((action.to_string(), input.clone()));
        match action {
            "resolve_workspace_ship_input" => Ok(json!({
                "mode": "pr",
                "base_branch": "agent-main",
            })),
            "drain_window" if input.get("deadline").is_none() => Ok(json!({
                "deadline": "2099-01-01T00:00:00Z",
                "expired": false,
                "remaining_seconds": 1,
            })),
            "drain_window" => {
                let reread = self.window_calls.fetch_add(1, Ordering::SeqCst);
                let expired = match self.scenario {
                    WorkspaceAutoScenario::MixedThenLater => reread >= 1,
                    WorkspaceAutoScenario::TerminalNonSuccesses
                    | WorkspaceAutoScenario::DispatchFailure
                    | WorkspaceAutoScenario::MalformedResult
                    | WorkspaceAutoScenario::ClassifierFailure => true,
                };
                Ok(json!({
                    "deadline": "2099-01-01T00:00:00Z",
                    "expired": expired,
                    "remaining_seconds": usize::from(!expired),
                }))
            }
            "classify_workspace_auto_tasks"
                if matches!(self.scenario, WorkspaceAutoScenario::ClassifierFailure) =>
            {
                self.classify_calls.fetch_add(1, Ordering::SeqCst);
                Err(DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: "fixture workspace classification failed".to_string(),
                })
            }
            "classify_workspace_auto_tasks" => Ok(self.classification()),
            "invoke_and_wait" => self.invoke_result(input),
            "pipeline_success_guard" => <OrbitRuntime as RuntimeHost>::run_deterministic(
                self.runtime,
                action,
                config,
                input,
                tool_context,
            ),
            other => Err(DispatchError::DeterministicActionNotRegistered(
                other.to_string(),
            )),
        }
    }

    fn resolve_cli_executor(&self, provider: &str) -> Result<ResolvedCliExecutor, DispatchError> {
        <OrbitRuntime as RuntimeHost>::resolve_cli_executor(self.runtime, provider)
    }

    fn tool_context_for_activity(
        &self,
        run_id: Option<&str>,
        fs_profile: Option<&str>,
        fs_audit: Option<Arc<dyn FsAuditLogger>>,
        proc_allowed_programs: Option<&[String]>,
    ) -> ToolContext {
        <OrbitRuntime as RuntimeHost>::tool_context_for_activity(
            self.runtime,
            run_id,
            fs_profile,
            fs_audit,
            proc_allowed_programs,
        )
    }
}

#[test]
fn workspace_auto_records_failed_leaf_and_dispatches_later_work() {
    let (_root, runtime, repo_root, global_root) = test_runtime();
    seed_default_catalogs(&global_root);
    let failed_task_id = seed_gate_task(&runtime, &repo_root, TaskStatus::Blocked);
    let host = ScriptedWorkspaceAutoHost::new(
        &runtime,
        WorkspaceAutoScenario::MixedThenLater,
        failed_task_id.clone(),
    );
    let input = json!({
        "max_tasks": 50,
        "for_seconds": 10,
        "idle_sleep_seconds": 0,
    });
    let run = runtime
        .stores()
        .jobs()
        .insert_job_run(
            "workspace_auto_pipeline",
            1,
            Utc::now(),
            Some(input.clone()),
            None,
        )
        .expect("insert workspace auto run");
    runtime
        .write_run_state(
            &run.run_id,
            &PipelineState::new(run.run_id.clone(), run.job_id.clone(), input.clone()),
        )
        .expect("write workspace auto run state");

    let outcome = try_execute_named_job(
        &runtime,
        &repo_root,
        &host,
        "workspace_auto_pipeline",
        input,
        &run.run_id,
    )
    .expect("mixed leaf outcomes must not fail the workspace drain");

    assert!(outcome.success);
    assert_eq!(host.classify_calls.load(Ordering::SeqCst), 2);
    let invokes = host.inputs_for("invoke_and_wait");
    assert_eq!(invokes.len(), 3);
    assert!(
        invokes
            .iter()
            .any(|input| { input["run_input"]["task_ids"] == json!(["ORB-LATER"]) })
    );

    let state = runtime
        .read_run_state(&run.run_id)
        .expect("read workspace auto run state")
        .expect("workspace auto run state exists");
    let failed_dispatch = state
        .child_dispatches
        .iter()
        .find(|dispatch| dispatch.child_run_id == "jrun-scripted-failed-leaf")
        .expect("failed child remains linked in parent run state");
    assert_eq!(
        failed_dispatch.parent_step_id.as_deref(),
        Some("leaf_invoke")
    );
    assert_eq!(failed_dispatch.phase, ChildDispatchPhase::Terminal);
    assert_eq!(failed_dispatch.child_status.as_deref(), Some("failed"));
    assert_eq!(
        failed_dispatch.error.as_deref(),
        Some("fixture leaf implementation failed")
    );
    let audit_events = runtime
        .list_audit_events(None, None, Some(AuditEventStatus::Success), None, 64)
        .expect("list workspace auto audit events");
    let failed_batch = audit_events
        .iter()
        .filter(|event| event.command == "pipeline.child_results")
        .filter_map(|event| event.arguments_json.as_deref())
        .filter_map(|payload| serde_json::from_str::<Value>(payload).ok())
        .find(|payload| payload["non_success_count"] == json!(1))
        .expect("failed batch aggregate remains in durable audit history");
    assert_eq!(failed_batch["checked_count"], 2);
    assert_eq!(failed_batch["succeeded_count"], 1);
    assert_eq!(
        failed_batch["results"],
        json!([
            {
                "run_id": "jrun-scripted-succeeded-leaf",
                "status": "succeeded"
            },
            {
                "run_id": "jrun-scripted-failed-leaf",
                "status": "failed",
                "error": "fixture leaf implementation failed"
            }
        ])
    );
    assert_eq!(
        runtime
            .get_task(&failed_task_id)
            .expect("show failed child task")
            .status,
        TaskStatus::Blocked,
        "the workspace parent must not reset or requeue the failed child task"
    );
}

#[test]
fn workspace_auto_records_every_terminal_non_success_without_failing() {
    let (_root, runtime, repo_root, global_root) = test_runtime();
    seed_default_catalogs(&global_root);
    let host = ScriptedWorkspaceAutoHost::new(
        &runtime,
        WorkspaceAutoScenario::TerminalNonSuccesses,
        "ORB-UNUSED",
    );
    let input = json!({
        "max_tasks": 50,
        "for_seconds": 0,
        "idle_sleep_seconds": 0,
    });
    let run = runtime
        .stores()
        .jobs()
        .insert_job_run(
            "workspace_auto_pipeline",
            1,
            Utc::now(),
            Some(input.clone()),
            None,
        )
        .expect("insert workspace auto run");
    runtime
        .write_run_state(
            &run.run_id,
            &PipelineState::new(run.run_id.clone(), run.job_id.clone(), input.clone()),
        )
        .expect("write workspace auto run state");

    let outcome = try_execute_named_job(
        &runtime,
        &repo_root,
        &host,
        "workspace_auto_pipeline",
        input,
        &run.run_id,
    )
    .expect("terminal child outcomes are data for the workspace sequencer");

    assert!(outcome.success);
    assert_eq!(host.classify_calls.load(Ordering::SeqCst), 1);
    let state = runtime
        .read_run_state(&run.run_id)
        .expect("read workspace auto run state")
        .expect("workspace auto run state exists");
    let observed = state
        .child_dispatches
        .iter()
        .map(|dispatch| {
            (
                dispatch.child_run_id.as_str(),
                dispatch.child_status.as_deref(),
                dispatch.error.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(observed.len(), 4);
    for expected in [
        (
            "jrun-scripted-failed",
            Some("failed"),
            Some("fixture failed"),
        ),
        (
            "jrun-scripted-cancelled",
            Some("cancelled"),
            Some("fixture cancelled"),
        ),
        (
            "jrun-scripted-interrupted",
            Some("interrupted"),
            Some("fixture interrupted"),
        ),
        (
            "jrun-scripted-timeout",
            Some("timeout"),
            Some("fixture timeout"),
        ),
    ] {
        assert!(
            observed.contains(&expected),
            "missing child outcome {expected:?}"
        );
    }
    let audit_events = runtime
        .list_audit_events(None, None, Some(AuditEventStatus::Success), None, 64)
        .expect("list workspace auto audit events");
    let batch = audit_events
        .iter()
        .filter(|event| event.command == "pipeline.child_results")
        .filter_map(|event| event.arguments_json.as_deref())
        .filter_map(|payload| serde_json::from_str::<Value>(payload).ok())
        .find(|payload| payload["non_success_count"] == json!(4))
        .expect("terminal non-success aggregate is durable");
    assert_eq!(batch["checked_count"], 4);
    assert_eq!(batch["succeeded_count"], 0);
    assert_eq!(batch["results"].as_array().expect("batch results").len(), 4);
}

#[test]
fn workspace_auto_fails_promptly_when_leaf_dispatch_has_no_durable_child() {
    let (_root, runtime, repo_root, global_root) = test_runtime();
    seed_default_catalogs(&global_root);
    let host = ScriptedWorkspaceAutoHost::new(
        &runtime,
        WorkspaceAutoScenario::DispatchFailure,
        "ORB-UNUSED",
    );

    let err = try_execute_named_job(
        &runtime,
        &repo_root,
        &host,
        "workspace_auto_pipeline",
        json!({"max_tasks": 50, "for_seconds": 0, "idle_sleep_seconds": 0}),
        "jrun-workspace-auto-dispatch-failure",
    )
    .expect_err("pre-link dispatch failure must fail the workspace drain");

    let message = err.to_string();
    assert!(
        message.contains("fixture dispatch failed before durable child creation"),
        "{message}"
    );
    assert_eq!(host.classify_calls.load(Ordering::SeqCst), 1);
    assert!(host.inputs_for("pipeline_success_guard").is_empty());
}

#[test]
fn workspace_auto_fails_closed_on_malformed_collected_leaf_result() {
    let (_root, runtime, repo_root, global_root) = test_runtime();
    seed_default_catalogs(&global_root);
    let host = ScriptedWorkspaceAutoHost::new(
        &runtime,
        WorkspaceAutoScenario::MalformedResult,
        "ORB-UNUSED",
    );

    let err = try_execute_named_job(
        &runtime,
        &repo_root,
        &host,
        "workspace_auto_pipeline",
        json!({"max_tasks": 50, "for_seconds": 0, "idle_sleep_seconds": 0}),
        "jrun-workspace-auto-malformed-result",
    )
    .expect_err("malformed collected result must fail the workspace drain");

    let message = err.to_string();
    assert!(
        message.contains("missing non-empty string run_id"),
        "{message}"
    );
    assert_eq!(host.classify_calls.load(Ordering::SeqCst), 1);
    assert_eq!(host.inputs_for("pipeline_success_guard").len(), 1);
}

#[test]
fn workspace_auto_preserves_concrete_workspace_step_failure() {
    let (_root, runtime, repo_root, global_root) = test_runtime();
    seed_default_catalogs(&global_root);
    let host = ScriptedWorkspaceAutoHost::new(
        &runtime,
        WorkspaceAutoScenario::ClassifierFailure,
        "ORB-UNUSED",
    );

    let err = try_execute_named_job(
        &runtime,
        &repo_root,
        &host,
        "workspace_auto_pipeline",
        json!({"max_tasks": 50, "for_seconds": 0, "idle_sleep_seconds": 0}),
        "jrun-workspace-auto-classifier-failure",
    )
    .expect_err("workspace-level deterministic failure must fail the drain");

    let message = err.to_string();
    assert!(
        message.contains("fixture workspace classification failed"),
        "{message}"
    );
    assert_eq!(host.classify_calls.load(Ordering::SeqCst), 1);
    assert!(host.inputs_for("invoke_and_wait").is_empty());
}
