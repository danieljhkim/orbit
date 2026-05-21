use std::time::{Duration, Instant};

use super::super::support::*;
use serde_json::json;

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
