use std::time::{Duration, Instant};

use orbit_core::OrbitRuntime;
use serde_json::json;

use super::super::duel::*;
use super::super::support::dispatch_workflow;

fn duel_plan_args(base: Option<&str>, wait: bool) -> DuelPlanCommand {
    DuelPlanCommand {
        task_id: "T20260425-2010".to_string(),
        base: base.map(str::to_string),
        wait,
        json: false,
        planner_a: None,
        planner_b: None,
        arbiter: None,
    }
}

#[test]
fn duel_plan_uses_explicit_base_when_flag_set() {
    let plan = build_duel_plan_run_plan(
        &duel_plan_args(Some("main"), false),
        "agent-main",
        &[
            "codex".to_string(),
            "claude".to_string(),
            "gemini".to_string(),
            "grok".to_string(),
        ],
    )
    .expect("build duel-plan run plan");

    assert_eq!(plan.workflow_alias, "duel-plan");
    assert_eq!(
        plan.input,
        json!({
            "task_id": "T20260425-2010",
            "task_ids": ["T20260425-2010"],
            "base_branch": "main",
        })
    );
}

#[test]
fn duel_plan_falls_back_to_config_base_when_flag_absent() {
    let plan = build_duel_plan_run_plan(
        &duel_plan_args(None, false),
        "agent-main",
        &[
            "codex".to_string(),
            "claude".to_string(),
            "gemini".to_string(),
            "grok".to_string(),
        ],
    )
    .expect("build duel-plan run plan");

    assert_eq!(
        plan.input,
        json!({
            "task_id": "T20260425-2010",
            "task_ids": ["T20260425-2010"],
            "base_branch": "agent-main",
        })
    );
}

#[test]
fn duel_plan_dispatch_defaults_to_non_blocking() {
    let plan = build_duel_plan_run_plan(
        &duel_plan_args(None, false),
        "agent-main",
        &[
            "codex".to_string(),
            "claude".to_string(),
            "gemini".to_string(),
            "grok".to_string(),
        ],
    )
    .expect("build duel-plan run plan");

    assert!(!plan.wait_for_completion);
}

#[test]
fn duel_plan_wait_flag_requests_blocking_dispatch() {
    let plan = build_duel_plan_run_plan(
        &duel_plan_args(None, true),
        "agent-main",
        &[
            "codex".to_string(),
            "claude".to_string(),
            "gemini".to_string(),
            "grok".to_string(),
        ],
    )
    .expect("build duel-plan run plan");

    assert!(plan.wait_for_completion);
}

#[test]
fn default_duel_plan_dispatch_returns_submitted_run_identity() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let jobs_dir = runtime.data_root().join("resources/jobs");
    std::fs::create_dir_all(&jobs_dir).expect("create jobs dir");
    std::fs::write(
        jobs_dir.join("job_duel_plan_pipeline.yaml"),
        r#"schemaVersion: 2
kind: Job
metadata:
  name: job_duel_plan_pipeline
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
    .expect("write job_duel_plan_pipeline fixture");
    let plan = build_duel_plan_run_plan(
        &duel_plan_args(None, false),
        "agent-main",
        &[
            "codex".to_string(),
            "claude".to_string(),
            "gemini".to_string(),
            "grok".to_string(),
        ],
    )
    .expect("build duel-plan run plan");

    let started = Instant::now();
    let runs = dispatch_workflow(
        &runtime,
        plan.workflow_alias,
        &plan.input,
        false,
        plan.wait_for_completion,
        1,
    )
    .expect("dispatch duel-plan workflow");

    assert!(
        started.elapsed() < Duration::from_secs(1),
        "default duel-plan dispatch waited too long"
    );
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].workflow_alias, "duel-plan");
    assert_eq!(runs[0].job_id, "job_duel_plan_pipeline");
    assert!(matches!(runs[0].state.as_str(), "submitted" | "queued"));
    assert_eq!(runs[0].attempt, 1);
    assert!(runs[0].error_code.is_none());
    assert!(runs[0].error_message.is_none());
}

fn full_candidates() -> Vec<String> {
    vec![
        "codex".to_string(),
        "claude".to_string(),
        "gemini".to_string(),
        "grok".to_string(),
    ]
}

#[test]
fn duel_plan_explicit_all_three_populates_family_overrides_in_input() {
    let mut args = duel_plan_args(None, true);
    args.planner_a = Some("gemini".to_string());
    args.planner_b = Some("codex".to_string());
    args.arbiter = Some("grok".to_string());

    let plan = build_duel_plan_run_plan(&args, "main", &full_candidates())
        .expect("explicit roles build");

    assert_eq!(plan.input["planner_a_family"], "gemini");
    assert_eq!(plan.input["planner_b_family"], "codex");
    assert_eq!(plan.input["arbiter_family"], "grok");
    assert!(
        !plan
            .input
            .as_object()
            .unwrap()
            .contains_key("planner_a_agent_cli")
    ); // no roles yet
}

#[test]
fn duel_plan_explicit_partial_flags_errors_with_missing_names() {
    let mut args = duel_plan_args(None, false);
    args.planner_a = Some("gemini".to_string());
    // planner_b and arbiter absent
    let err = build_duel_plan_run_plan(&args, "main", &full_candidates()).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("missing --planner-b, --arbiter"), "msg={msg}");
}

#[test]
fn duel_plan_explicit_invalid_family_errors_with_expected_list() {
    let mut args = duel_plan_args(None, false);
    args.planner_a = Some("xyz".to_string());
    args.planner_b = Some("codex".to_string());
    args.arbiter = Some("grok".to_string());
    let err = build_duel_plan_run_plan(&args, "main", &full_candidates()).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("unknown agent family 'xyz'"), "msg={msg}");
    assert!(msg.contains("codex, claude, gemini, grok"), "msg={msg}");
}

#[test]
fn duel_plan_explicit_family_not_in_candidates_errors_with_name_and_list() {
    let mut args = duel_plan_args(None, false);
    args.planner_a = Some("gemini".to_string());
    args.planner_b = Some("codex".to_string());
    args.arbiter = Some("claude".to_string());
    let cands = vec![
        "codex".to_string(),
        "gemini".to_string(),
        "grok".to_string(),
    ];
    let err = build_duel_plan_run_plan(&args, "main", &cands).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("claude"), "msg={msg}");
    assert!(msg.contains("candidates"), "msg={msg}");
}

#[test]
fn duel_plan_explicit_duplicate_family_errors_with_duplicated_name() {
    let mut args = duel_plan_args(None, false);
    args.planner_a = Some("gemini".to_string());
    args.planner_b = Some("gemini".to_string());
    args.arbiter = Some("codex".to_string());
    let err = build_duel_plan_run_plan(&args, "main", &full_candidates()).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("'gemini' appears more than once"), "msg={msg}");
}

#[test]
fn duel_plan_no_explicit_flags_omits_family_keys_from_input() {
    let plan =
        build_duel_plan_run_plan(&duel_plan_args(None, false), "main", &full_candidates())
            .expect("no-override");

    let obj = plan.input.as_object().unwrap();
    assert!(!obj.contains_key("planner_a_family"));
    assert!(!obj.contains_key("planner_b_family"));
    assert!(!obj.contains_key("arbiter_family"));
}
