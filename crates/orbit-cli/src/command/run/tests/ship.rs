use serde_json::json;

use crate::command::Execute;
use orbit_core::{OrbitError, OrbitRuntime};

use super::super::ship::*;

fn ship_args(task_ids: &[&str], mode: ShipMode, base: Option<&str>) -> ShipCommand {
    ShipCommand {
        task_ids: task_ids.iter().map(|value| value.to_string()).collect(),
        mode: Some(mode),
        base: base.map(str::to_string),
        review: false,
        review_crew: None,
        json: false,
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
        &ship_args(&["ORB-00001", "ORB-00002"], ShipMode::Pr, None),
        "agent-main",
    )
    .expect("build plan");

    assert_eq!(plan.workflow_alias, SHIP_WORKFLOW);
    assert_eq!(
        plan.input,
        json!({
            "mode": "pr",
            "base_branch": "agent-main",
            "task_ids": ["ORB-00001", "ORB-00002"],
        })
    );
}

#[test]
fn explicit_ship_preserves_local_mode_and_base_override() {
    let plan = build_plan(
        &ship_args(&["ORB-00001"], ShipMode::Local, Some("main")),
        "agent-main",
    )
    .expect("build plan");

    assert_eq!(plan.workflow_alias, SHIP_WORKFLOW);
    assert_eq!(
        plan.input,
        json!({
            "mode": "local",
            "base_branch": "main",
            "task_ids": ["ORB-00001"],
        })
    );
}

#[test]
fn ship_threads_explicit_review_controls() {
    let mut args = ship_args(&["ORB-00001"], ShipMode::Pr, None);
    args.review = true;
    args.review_crew = Some("opus-review".to_string());

    let plan = build_plan(&args, "agent-main").expect("build review plan");

    assert_eq!(plan.input["review"], true);
    assert_eq!(plan.input["review_crew"], "opus-review");
}

#[test]
fn ship_rejects_enabled_review_without_explicit_crew() {
    let mut args = ship_args(&["ORB-00001"], ShipMode::Pr, None);
    args.review = true;

    let error = build_plan(&args, "agent-main").expect_err("review crew is required");
    assert!(
        error.to_string().contains("non-blank explicit review crew"),
        "unexpected error: {error}"
    );
}

#[test]
fn ship_local_deprecation_returns_legacy_error() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let err = LegacyShipLocalCommand {
        task_ids: vec!["ORB-00001".to_string()],
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
        err.to_string().contains("orbit run ship"),
        "unexpected error: {err}"
    );
}

#[test]
fn ship_rejects_duplicate_task_ids() {
    let err = build_plan(
        &ship_args(&["ORB-00001", "ORB-00001"], ShipMode::Pr, None),
        "agent-main",
    )
    .expect_err("duplicate task IDs should fail");
    assert!(
        err.to_string().contains("duplicate task id"),
        "unexpected error: {err}"
    );
}
