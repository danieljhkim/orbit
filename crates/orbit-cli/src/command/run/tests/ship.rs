use crate::command::Execute;
use orbit_core::application::task::TaskAddParams;
use orbit_core::{OrbitError, OrbitRuntime, TaskStatus};
use serde_json::json;

use super::super::ship::*;

fn ship_args(task_ids: &[&str], mode: ShipMode, base: Option<&str>) -> ShipCommand {
    ShipCommand {
        task_ids: task_ids.iter().map(|value| value.to_string()).collect(),
        mode: Some(mode),
        base: base.map(str::to_string),
        json: false,
        claim_token: None,
    }
}

/// Build a ship plan from test args, threading the args' explicit mode through
/// the resolved-mode parameter (production resolves this from the registry).
fn build_plan(args: &ShipCommand, config_base_branch: &str) -> Result<WorkflowRunPlan, OrbitError> {
    let mode = args.mode.expect("test args set an explicit mode").to_core();
    build_ship_run_plan(args, config_base_branch, mode)
}

#[test]
fn ship_auto_mode_omits_task_ids_and_uses_pr_mode_by_default() {
    let plan = build_plan(&ship_args(&[], ShipMode::Pr, None), "agent-main").expect("build plan");

    assert_eq!(plan.workflow_alias, SHIP_WORKFLOW);
    assert_eq!(
        plan.input,
        json!({
            "mode": "pr",
            "base_branch": "agent-main",
        })
    );
}

#[test]
fn ship_auto_mode_preserves_local_mode_and_base_override() {
    let plan = build_plan(&ship_args(&[], ShipMode::Local, Some("main")), "agent-main")
        .expect("build plan");

    assert_eq!(plan.workflow_alias, SHIP_WORKFLOW);
    assert_eq!(
        plan.input,
        json!({
            "mode": "local",
            "base_branch": "main",
        })
    );
}

#[test]
fn explicit_ship_uses_unified_gated_workflow_with_pr_mode() {
    let plan = build_plan(
        &ship_args(&["T20260425-2010", "T20260425-2011"], ShipMode::Pr, None),
        "agent-main",
    )
    .expect("build plan");

    assert_eq!(plan.workflow_alias, SHIP_WORKFLOW);
    assert_eq!(
        plan.input,
        json!({
            "mode": "pr",
            "base_branch": "agent-main",
            "task_ids": ["T20260425-2010", "T20260425-2011"],
        })
    );
}

#[test]
fn explicit_ship_preserves_local_mode_and_base_override() {
    let plan = build_plan(
        &ship_args(&["T20260425-2010"], ShipMode::Local, Some("main")),
        "agent-main",
    )
    .expect("build plan");

    assert_eq!(plan.workflow_alias, SHIP_WORKFLOW);
    assert_eq!(
        plan.input,
        json!({
            "mode": "local",
            "base_branch": "main",
            "task_ids": ["T20260425-2010"],
        })
    );
}

#[test]
fn ship_local_deprecation_returns_legacy_error() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let err = LegacyShipLocalCommand {
        task_ids: vec!["T20260425-2010".to_string()],
        base: None,
        json: false,
    }
    .execute(&runtime)
    .expect_err("deprecated command should fail");
    assert!(
        err.to_string().contains("orbit run ship --mode local"),
        "unexpected error: {err}"
    );
}

#[test]
fn ship_rejects_removed_history_forms() {
    let err = build_plan(&ship_args(&["list"], ShipMode::Pr, None), "agent-main")
        .expect_err("legacy history form should fail");
    assert!(
        err.to_string().contains("orbit run history"),
        "unexpected error: {err}"
    );

    let err = build_plan(&ship_args(&["show"], ShipMode::Pr, None), "agent-main")
        .expect_err("legacy history form should fail");
    assert!(
        err.to_string().contains("orbit run history"),
        "unexpected error: {err}"
    );
}

#[test]
fn ship_rejects_removed_local_subcommand_form() {
    let err = build_plan(&ship_args(&["local"], ShipMode::Pr, None), "agent-main")
        .expect_err("legacy local form should fail");
    assert!(
        err.to_string().contains("--mode local"),
        "unexpected error: {err}"
    );
}

#[test]
fn ship_rejects_removed_auto_positional_form() {
    let err = build_plan(&ship_args(&["auto"], ShipMode::Pr, None), "agent-main")
        .expect_err("legacy auto form should fail");
    assert!(
        err.to_string().contains("orbit run auto"),
        "unexpected error: {err}"
    );
}

#[test]
fn ship_rejects_duplicate_task_ids() {
    let err = build_plan(
        &ship_args(&["T20260425-2010", "T20260425-2010"], ShipMode::Pr, None),
        "agent-main",
    )
    .expect_err("duplicate task IDs should fail");
    assert!(
        err.to_string().contains("duplicate task id"),
        "unexpected error: {err}"
    );
}

fn write_ship_job_asset(runtime: &OrbitRuntime) {
    let jobs_dir = runtime.data_root().join("resources/jobs");
    std::fs::create_dir_all(&jobs_dir).expect("create jobs directory");
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
    - id: wait
      spec:
        type: deterministic
        action: sleep
        config:
          seconds: 30
"#,
    )
    .expect("write ship job fixture");
}

/// The interactive command must enter the same runtime submission path as the
/// dashboard, MCP tool, and routine action. A generic workflow dispatch would
/// insert a second run here instead of returning this typed shared conflict.
#[test]
fn interactive_ship_inherits_the_shared_in_flight_guard() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    write_ship_job_asset(&runtime);
    let task = runtime
        .add_task(TaskAddParams {
            title: "CLI ship admission fixture".to_string(),
            description: "Synthetic task for the shared in-flight guard".to_string(),
            status: Some(TaskStatus::Backlog),
            ..TaskAddParams::default()
        })
        .expect("create task fixture");
    let task_id = task.id.to_string();
    ship_args(&[&task_id], ShipMode::Local, Some("main"))
        .execute(&runtime)
        .expect("first interactive ship dispatch");
    let first_run = runtime
        .list_job_runs(Default::default())
        .expect("list first CLI run")
        .into_iter()
        .next()
        .expect("first CLI dispatch persists a run");
    assert_eq!(
        first_run.input,
        Some(json!({
            "mode": "local",
            "base_branch": "main",
            "task_ids": [task_id],
        }))
    );
    let audits = runtime
        .list_audit_events(None, None, None, None, 20)
        .expect("list CLI submission audits");
    assert!(audits.iter().any(|audit| {
        audit.tool_name.as_deref() == Some("pipeline.invoke")
            && audit.target_id.as_deref() == Some(first_run.run_id.as_str())
    }));

    let error = ship_args(&[&task_id], ShipMode::Local, Some("main"))
        .execute(&runtime)
        .expect_err("an interactive ship must refuse a task already in flight");

    let OrbitError::ShipRunInFlight { task_id, run_id } = &error else {
        panic!("expected ShipRunInFlight, got {error:?}");
    };
    assert_eq!(task_id, &task.id.to_string());
    assert!(run_id.starts_with("jrun-"), "unexpected run id: {run_id}");

    let runs = runtime
        .list_job_runs(Default::default())
        .expect("list job runs");
    assert_eq!(
        runs.len(),
        1,
        "a refused CLI submission must not insert a run"
    );
}
