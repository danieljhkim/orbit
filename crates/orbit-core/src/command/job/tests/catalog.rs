use super::super::catalog::JobCatalogFilter;

use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::activity_job::{V2ActivityCatalog, resolve_job_target_refs};
use orbit_common::types::{
    ActivityV2Spec, JobRunState, JobV2, JobV2Step, JobV2StepBody, PipelineState,
    load_activity_asset, load_job_asset,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use tempfile::tempdir;

use crate::OrbitRuntime;
use crate::command::activity::DEFAULT_ACTIVITY_FILES;

const DEFAULT_JOB_FILES: &[(&str, &str)] = &[
    (
        "auto_task_scheduler_pipeline",
        include_str!("../../../../assets/jobs/auto_task_scheduler_pipeline.yaml"),
    ),
    (
        "task_auto_pipeline",
        include_str!("../../../../assets/jobs/task_auto_pipeline.yaml"),
    ),
    (
        "task_gate_pipeline",
        include_str!("../../../../assets/jobs/task_gate_pipeline.yaml"),
    ),
    (
        "task_local_pipeline",
        include_str!("../../../../assets/jobs/task_local_pipeline.yaml"),
    ),
    (
        "task_pilot_pipeline",
        include_str!("../../../../assets/jobs/task_pilot_pipeline.yaml"),
    ),
    (
        "task_pr_pipeline",
        include_str!("../../../../assets/jobs/task_pr_pipeline.yaml"),
    ),
    (
        "task_triage_pipeline",
        include_str!("../../../../assets/jobs/task_triage_pipeline.yaml"),
    ),
    (
        "workspace_ship_pipeline",
        include_str!("../../../../assets/jobs/workspace_ship_pipeline.yaml"),
    ),
    (
        "worktree_gc_pipeline",
        include_str!("../../../../assets/jobs/worktree_gc_pipeline.yaml"),
    ),
];

fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf, PathBuf) {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
    (root, runtime, global_root, workspace_root)
}

fn write_job(path: &Path, name: &str, action: &str, max_active_runs: u32) {
    let yaml = format!(
        r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  max_active_runs: {max_active_runs}
  steps:
    - id: marker
      spec:
        type: deterministic
        action: {action}
        config: {{}}
"#
    );
    std::fs::create_dir_all(path.parent().expect("job path has parent")).expect("create job dir");
    std::fs::write(path, yaml).expect("write job yaml");
}

fn write_empty_job(path: &Path, name: &str) {
    let yaml = format!(
        r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  max_active_runs: 1
  steps: []
"#
    );
    std::fs::create_dir_all(path.parent().expect("job path has parent")).expect("create job dir");
    std::fs::write(path, yaml).expect("write job yaml");
}

/// A workspace-local shadow job whose single step dispatches an unregistered
/// deterministic action. If catalog resolution incorrectly executed this
/// shadow instead of the builtin, the dispatch would fail
/// (`DeterministicActionNotRegistered`) and the run would land in `Failed` —
/// so a `Success` run proves the shadow's steps never ran.
fn write_failing_shadow_job(path: &Path, name: &str) {
    let yaml = format!(
        r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  max_active_runs: 1
  steps:
    - id: exploit
      spec:
        type: deterministic
        description: Unregistered action; must never be dispatched.
        action: __shadow_should_not_run__
        config: {{}}
"#
    );
    std::fs::create_dir_all(path.parent().expect("job path has parent")).expect("create job dir");
    std::fs::write(path, yaml).expect("write job yaml");
}

#[test]
fn workspace_default_named_job_does_not_run_when_workflow_invoked_by_name() {
    let (_root, runtime, global_root, workspace_root) = test_runtime();
    let job_name = "task_auto_pipeline";
    let global_job = global_root.join("resources/jobs/task_auto_pipeline.yaml");
    let workspace_job = workspace_root.join("resources/jobs/task_auto_pipeline.yaml");
    write_empty_job(&global_job, job_name);
    write_failing_shadow_job(&workspace_job, job_name);

    let run = runtime
        .stores()
        .jobs()
        .insert_job_run(job_name, 1, Utc::now(), Some(json!({})), None)
        .expect("insert named pipeline run");
    runtime
        .stores()
        .jobs()
        .write_run_state(
            &run.run_id,
            &PipelineState::new(run.run_id.clone(), run.job_id.clone(), json!({})),
        )
        .expect("write initial pipeline state");

    runtime
        .execute_pipeline_run_worker(&run.run_id)
        .expect("execute named pipeline run");

    let finished = runtime
        .show_job_run(&run.run_id)
        .expect("show finished run");
    assert_eq!(
        finished.state,
        JobRunState::Success,
        "workspace-local job shadow must not execute; the empty builtin should run instead"
    );
}

fn default_activity_catalog() -> V2ActivityCatalog {
    let mut catalog = V2ActivityCatalog::new();
    for (name, yaml) in DEFAULT_ACTIVITY_FILES {
        let asset = load_activity_asset(yaml)
            .unwrap_or_else(|err| panic!("default activity {name} should parse: {err}"));
        assert_eq!(&asset.name, name);
        catalog.insert(*name, asset.spec);
    }
    catalog
}

#[test]
fn seeded_step_failure_recovery_asset_stays_aligned_without_retired_role() {
    let seeded = DEFAULT_ACTIVITY_FILES
        .iter()
        .find_map(|(name, yaml)| (*name == "step_failure_recovery").then_some(*yaml))
        .expect("seeded step failure recovery activity");
    assert_eq!(
        seeded,
        include_str!("../../../../../../.orbit/resources/activities/step_failure_recovery.yaml"),
        "dogfood and seeded recovery activity assets must remain behaviorally aligned"
    );

    let asset = load_activity_asset(seeded).expect("parse recovery activity");
    let ActivityV2Spec::AgentLoop(_) = asset.spec.spec else {
        panic!("step_failure_recovery must remain an agent loop");
    };
    assert!(!seeded.contains("\n  role:"));
}

fn assert_condition_tokens_are_paths(condition: &str) {
    let mut remaining = condition;
    while let Some(start) = remaining.find("{{") {
        let after_start = &remaining[start + 2..];
        let end = after_start
            .find("}}")
            .unwrap_or_else(|| panic!("unterminated template token in {condition:?}"));
        let token = after_start[..end].trim();
        assert!(
            !["==", "!=", "&&", "||", ">", "<"]
                .iter()
                .any(|op| token.contains(op)),
            "template token {token:?} in condition {condition:?} must be a path; put comparisons outside the braces",
        );
        remaining = &after_start[end + 2..];
    }
}

fn assert_step_condition_tokens_are_paths(step: &orbit_common::types::JobV2Step) {
    if let Some(when) = &step.when {
        assert_condition_tokens_are_paths(when);
    }
    match &step.body {
        JobV2StepBody::Parallel { parallel } => {
            for branch in &parallel.branches {
                assert_step_condition_tokens_are_paths(branch);
            }
        }
        JobV2StepBody::FanOut { fan_out, .. } => {
            assert_step_condition_tokens_are_paths(&fan_out.worker);
        }
        JobV2StepBody::Loop { loop_ } => {
            if let Some(break_when) = &loop_.break_when {
                assert_condition_tokens_are_paths(break_when);
            }
            for child in &loop_.steps {
                assert_step_condition_tokens_are_paths(child);
            }
        }
        JobV2StepBody::TargetRef(_) | JobV2StepBody::Target(_) => {}
    }
}

#[test]
fn default_job_target_refs_resolve_against_default_activities() {
    let catalog = default_activity_catalog();

    for (job_name, yaml) in DEFAULT_JOB_FILES {
        let mut asset = load_job_asset(yaml)
            .unwrap_or_else(|err| panic!("default job {job_name} should parse: {err}"));
        resolve_job_target_refs(&mut asset.spec, &catalog)
            .unwrap_or_else(|err| panic!("default job {job_name} refs resolve: {err}"));
    }
}

#[test]
fn task_pilot_pipeline_defaults_to_luna_and_bounded_all_join_partitions() {
    let yaml = DEFAULT_JOB_FILES
        .iter()
        .find_map(|(name, yaml)| (*name == "task_pilot_pipeline").then_some(*yaml))
        .expect("task pilot pipeline exists");
    let asset = load_job_asset(yaml).expect("task pilot pipeline parses");
    let defaults = asset.spec.default_input.as_ref().expect("default input");
    assert_eq!(defaults["task_ids"], json!([]));
    assert_eq!(defaults["crew"], "luna");
    assert_eq!(defaults["max_partition_size"], 5);
    assert_eq!(asset.spec.steps.len(), 3);

    let JobV2StepBody::TargetRef(prepare) = &asset.spec.steps[0].body else {
        panic!("task pilot preparation must be deterministic activity reference");
    };
    assert_eq!(prepare.target, "activity:prepare_task_pilot");
    let prepare_input = prepare.default_input.as_ref().expect("prepare input");
    assert_eq!(prepare_input["task_ids"], "{{ input.task_ids }}");

    let JobV2StepBody::FanOut { fan_out, fan_in } = &asset.spec.steps[1].body else {
        panic!("task pilot agent work must fan out");
    };
    assert_eq!(fan_out.items, "{{ steps.prepare.output.partitions }}");
    assert_eq!(fan_out.max_workers, 5);
    assert_eq!(
        fan_in.join,
        orbit_common::types::activity_job::JoinMode::All
    );
    assert_eq!(fan_in.collect.as_deref(), Some("pilot_results"));
    let JobV2StepBody::TargetRef(pilot) = &fan_out.worker.body else {
        panic!("task pilot worker must reference agent activity");
    };
    assert_eq!(pilot.target, "activity:task_pilot");
    let pilot_input = pilot.default_input.as_ref().expect("pilot input");
    assert_eq!(pilot_input["task_ids"], "{{ item.task_ids }}");
    assert_eq!(pilot_input["crew"], "{{ input.crew }}");

    let JobV2StepBody::TargetRef(apply) = &asset.spec.steps[2].body else {
        panic!("task pilot apply must be deterministic activity reference");
    };
    assert_eq!(apply.target, "activity:apply_task_pilot_results");
    let apply_input = apply.default_input.as_ref().expect("apply input");
    assert_eq!(apply_input["prepared"], "{{ steps.prepare.output }}");
    assert_eq!(apply_input["results"], "{{ steps.pilot_results.output }}");
    assert!(
        yaml.contains("Invoked-only"),
        "task pilot must remain an explicitly invoked workflow"
    );

    let mut resolved = load_job_asset(yaml).expect("task pilot pipeline parses for resolution");
    resolve_job_target_refs(&mut resolved.spec, &default_activity_catalog())
        .expect("task pilot activity references resolve");
    let JobV2StepBody::FanOut { fan_out, .. } = &resolved.spec.steps[1].body else {
        panic!("resolved task pilot agent work must remain a fan-out");
    };
    let JobV2StepBody::Target(pilot) = &fan_out.worker.body else {
        panic!("task pilot worker must resolve to an activity target");
    };
    assert_eq!(
        pilot.fs_profile.as_deref(),
        Some("reviewer"),
        "resolved task-pilot worker must preserve the read-only filesystem profile"
    );
}

/// [ORB-10385] Every deterministic action reachable from a shipped job —
/// including terminal `failure_activity` hooks — must be registered in
/// this binary's v2 dispatch table. `pr_failure_handoff` shipped as a
/// catalog asset bound to `task_pr_pipeline` while orbit-core's dispatch
/// arm still omitted it, so the hook fired as "deterministic action not
/// registered" on three runs, each after a task had been admitted,
/// implemented, and validated. `worktree_gc` had the same gap.
#[test]
fn default_jobs_only_reference_registered_deterministic_actions() {
    let (_root, runtime, _global_root, _workspace_root) = test_runtime();
    let catalog = default_activity_catalog();

    for (job_name, yaml) in DEFAULT_JOB_FILES {
        let mut asset = load_job_asset(yaml)
            .unwrap_or_else(|err| panic!("default job {job_name} should parse: {err}"));
        resolve_job_target_refs(&mut asset.spec, &catalog)
            .unwrap_or_else(|err| panic!("default job {job_name} refs resolve: {err}"));
        orbit_engine::validate_job_deterministic_actions(&asset.spec, &runtime).unwrap_or_else(
            |err| panic!("default job {job_name} references an unregistered action: {err}"),
        );
    }
}

/// Companion to the job sweep above: a seeded deterministic activity that
/// no shipped job targets yet must still be dispatchable, or the first job
/// to bind it inherits the same skew.
#[test]
fn default_deterministic_activities_are_registered_in_the_runtime() {
    let (_root, runtime, _global_root, _workspace_root) = test_runtime();

    for (name, yaml) in DEFAULT_ACTIVITY_FILES {
        let asset = load_activity_asset(yaml)
            .unwrap_or_else(|err| panic!("default activity {name} should parse: {err}"));
        let ActivityV2Spec::Deterministic(spec) = &asset.spec.spec else {
            continue;
        };
        assert!(
            orbit_engine::V2RuntimeHost::has_deterministic_action(&runtime, &spec.action),
            "seeded activity `{name}` names deterministic action `{}`, which this runtime cannot dispatch",
            spec.action
        );
    }
}

#[test]
fn local_task_pipeline_commits_before_merge_and_reconciles_with_local_base() {
    let yaml = DEFAULT_JOB_FILES
        .iter()
        .find_map(|(name, yaml)| (*name == "task_local_pipeline").then_some(*yaml))
        .expect("task local pipeline default exists");
    let asset = load_job_asset(yaml).expect("parse task local pipeline");
    let root_step_ids = asset
        .spec
        .steps
        .iter()
        .map(|step| step.id.as_str())
        .collect::<Vec<_>>();

    let commit_index = root_step_ids
        .iter()
        .position(|id| *id == "commit")
        .expect("task local pipeline has commit step");
    let merge_index = root_step_ids
        .iter()
        .position(|id| *id == "merge")
        .expect("task local pipeline has merge step");
    assert!(
        commit_index < merge_index,
        "task local pipeline must commit before merge"
    );

    let merge = asset
        .spec
        .steps
        .iter()
        .find(|step| step.id == "merge")
        .expect("task local pipeline has merge step");
    let JobV2StepBody::TargetRef(merge) = &merge.body else {
        panic!("task local pipeline merge must reference git_merge");
    };
    let merge_input = merge.default_input.as_ref().expect("merge input");
    assert_eq!(
        merge_input["base_sync"], "local",
        "an unpublished earlier merge must be a valid base for the next local merge"
    );
}

#[test]
fn task_shipment_implementers_pin_workspace_and_repo_roots_to_the_worktree() {
    for job_name in ["task_local_pipeline", "task_pr_pipeline"] {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == job_name).then_some(*yaml))
            .unwrap_or_else(|| panic!("default job {job_name} exists"));
        let asset =
            load_job_asset(yaml).unwrap_or_else(|error| panic!("parse {job_name}: {error}"));
        let implement_bundle = asset
            .spec
            .steps
            .iter()
            .find(|step| step.id == "implement_bundle")
            .expect("implement bundle");
        let JobV2StepBody::Loop { loop_ } = &implement_bundle.body else {
            panic!("{job_name} implement bundle must be a loop");
        };
        let JobV2StepBody::TargetRef(implement) = &loop_.steps[0].body else {
            panic!("{job_name} implement step must reference agent_implement");
        };
        let input = implement.default_input.as_ref().expect("implement input");
        for field in ["workspace_path", "repo_root"] {
            assert_eq!(
                input[field], "{{ steps.worktree.output.workspace_path }}",
                "{job_name} must pin {field} to the exact assigned worktree"
            );
        }
    }
}

#[test]
fn task_shipment_commit_steps_use_the_worktree_base_checkpoint() {
    for job_name in ["task_local_pipeline", "task_pr_pipeline"] {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == job_name).then_some(*yaml))
            .unwrap_or_else(|| panic!("default job {job_name} exists"));
        let asset =
            load_job_asset(yaml).unwrap_or_else(|error| panic!("parse {job_name}: {error}"));
        let commit = asset
            .spec
            .steps
            .iter()
            .find(|step| step.id == "commit")
            .expect("commit step");
        let JobV2StepBody::TargetRef(commit) = &commit.body else {
            panic!("{job_name} commit step must reference git_commit");
        };
        let input = commit.default_input.as_ref().expect("commit input");
        assert_eq!(
            input["base_ref"], "{{ steps.worktree.output.base_ref }}",
            "{job_name} must pass the exact worktree start-point ref"
        );
        // ORB-10380: the commit step reconciles history against the commit
        // pinned at setup, never the moving ref name.
        assert_eq!(
            input["base_sha"], "{{ steps.worktree.output.base_sha }}",
            "{job_name} must pin the commit step to the setup-time base commit"
        );
    }
}

#[test]
fn pr_pipeline_models_handoff_phases_as_ordered_activity_checkpoints() {
    let yaml = DEFAULT_JOB_FILES
        .iter()
        .find_map(|(name, yaml)| (*name == "task_pr_pipeline").then_some(*yaml))
        .expect("task pr pipeline default exists");
    let asset = load_job_asset(yaml).expect("parse task pr pipeline");
    let phases = asset
        .spec
        .steps
        .iter()
        .filter_map(|step| match &step.body {
            JobV2StepBody::TargetRef(target) => Some((
                step.id.as_str(),
                target.target.as_str(),
                step.recovery_activity.as_deref(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        phases,
        vec![
            ("worktree", "activity:worktree_setup", None),
            (
                "commit",
                "activity:git_commit",
                Some("step_failure_recovery")
            ),
            (
                "prepare_branch",
                "activity:pr_prepare",
                Some("step_failure_recovery")
            ),
            (
                "sync_base",
                "activity:git_rebase",
                Some("step_failure_recovery")
            ),
            ("push", "activity:git_push", Some("step_failure_recovery")),
            ("pr_open", "activity:pr_open", Some("step_failure_recovery")),
            (
                "promote_tasks",
                "activity:pr_promote",
                Some("step_failure_recovery")
            ),
            (
                "promote_no_diff",
                "activity:pr_promote",
                Some("step_failure_recovery")
            ),
        ]
    );

    let pr_open = asset
        .spec
        .steps
        .iter()
        .find(|step| step.id == "pr_open")
        .expect("PR open phase");
    let JobV2StepBody::TargetRef(target) = &pr_open.body else {
        panic!("PR open must reference a focused activity");
    };
    let input = target.default_input.as_ref().expect("PR open input");
    for hidden_phase in ["scope", "rewrite_performed", "expected_remote_sha"] {
        assert!(
            input.get(hidden_phase).is_none(),
            "pr_open must not embed earlier {hidden_phase} phase input"
        );
    }
}

#[test]
fn gate_pipeline_releases_reservation_before_child_success_guard() {
    let yaml = DEFAULT_JOB_FILES
        .iter()
        .find_map(|(name, yaml)| (*name == "task_gate_pipeline").then_some(*yaml))
        .expect("task gate pipeline default exists");
    let asset = load_job_asset(yaml).expect("parse task gate pipeline");
    let root_step_ids = asset
        .spec
        .steps
        .iter()
        .map(|step| step.id.as_str())
        .collect::<Vec<_>>();

    let dispatch_index = root_step_ids
        .iter()
        .position(|id| *id == "dispatch_child")
        .expect("task gate pipeline has child dispatch step");
    let release_index = root_step_ids
        .iter()
        .position(|id| *id == "release_reservation")
        .expect("task gate pipeline has reservation release step");
    let guard_index = root_step_ids
        .iter()
        .position(|id| *id == "require_child_success")
        .expect("task gate pipeline has child success guard step");
    assert!(
        dispatch_index < release_index,
        "reservation must release only after invoke_and_wait returns"
    );
    assert!(
        release_index < guard_index,
        "reservation must release before the child success guard can fail the run"
    );

    let dispatch = &asset.spec.steps[dispatch_index];
    match &dispatch.body {
        JobV2StepBody::TargetRef(target) => {
            assert_eq!(target.target, "activity:invoke_and_wait");
            let input = target.default_input.as_ref().expect("dispatch input");
            assert_eq!(
                input["job_name"],
                Value::String("task_{{ input.mode }}_pipeline".to_string())
            );
            assert_eq!(
                input["admission_task_ids"],
                Value::String("{{ input.task_ids }}".to_string())
            );
            assert_eq!(
                input["admission_workflow"],
                Value::String("worktree_setup".to_string())
            );
        }
        other => panic!("expected dispatch target ref, got {other:?}"),
    }

    let release = &asset.spec.steps[release_index];
    assert_eq!(
        release.when.as_deref(),
        Some(
            "{{ steps.dispatch_child.output.status }} != timeout && {{ steps.dispatch_child.output.status }} != pending && {{ steps.dispatch_child.output.status }} != running"
        )
    );
    match &release.body {
        JobV2StepBody::TargetRef(target) => {
            assert_eq!(target.target, "activity:release_locks");
            let input = target.default_input.as_ref().expect("release input");
            assert_eq!(
                input["reservation_id"],
                Value::String("{{ steps.reserve.output.reservation_id }}".to_string())
            );
        }
        other => panic!("expected release target ref, got {other:?}"),
    }

    let guard = &asset.spec.steps[guard_index];
    assert_eq!(
        guard.when.as_deref(),
        Some("{{ steps.reserve.output.reserved }} == true")
    );
    match &guard.body {
        JobV2StepBody::TargetRef(target) => {
            assert_eq!(target.target, "activity:pipeline_success_guard");
            let input = target.default_input.as_ref().expect("guard input");
            assert_eq!(
                input["result"],
                Value::String("{{ steps.dispatch_child.output }}".to_string())
            );
        }
        other => panic!("expected guard target ref, got {other:?}"),
    }
}

#[test]
fn auto_pipeline_checks_gate_results_after_fan_in() {
    let yaml = DEFAULT_JOB_FILES
        .iter()
        .find_map(|(name, yaml)| (*name == "task_auto_pipeline").then_some(*yaml))
        .expect("task auto pipeline default exists");
    let asset = load_job_asset(yaml).expect("parse task auto pipeline");
    let root_step_ids = asset
        .spec
        .steps
        .iter()
        .map(|step| step.id.as_str())
        .collect::<Vec<_>>();

    let dispatch_index = root_step_ids
        .iter()
        .position(|id| *id == "dispatch")
        .expect("task auto pipeline has dispatch fan-out");
    let guard_index = root_step_ids
        .iter()
        .position(|id| *id == "require_gate_success")
        .expect("task auto pipeline has gate success guard");
    assert!(
        dispatch_index < guard_index,
        "gate results must be collected before the success guard runs"
    );

    let dispatch = &asset.spec.steps[dispatch_index];
    match &dispatch.body {
        JobV2StepBody::FanOut { fan_out, .. } => {
            assert_eq!(fan_out.max_workers, 5);
        }
        other => panic!("expected dispatch fan-out, got {other:?}"),
    }

    let guard = &asset.spec.steps[guard_index];
    assert_eq!(
        guard.when.as_deref(),
        Some("{{ steps.validate_bundles.output.bundle_count }} != 0")
    );
    match &guard.body {
        JobV2StepBody::TargetRef(target) => {
            assert_eq!(target.target, "activity:pipeline_success_guard");
            let input = target.default_input.as_ref().expect("guard input");
            assert_eq!(
                input["results"],
                Value::String("{{ steps.gate_results.output }}".to_string())
            );
        }
        other => panic!("expected guard target ref, got {other:?}"),
    }
}

#[test]
fn gate_pipeline_default_reservation_ttl_covers_child_wait_budget() {
    let yaml = DEFAULT_JOB_FILES
        .iter()
        .find_map(|(name, yaml)| (*name == "task_gate_pipeline").then_some(*yaml))
        .expect("task gate pipeline default exists");
    let asset = load_job_asset(yaml).expect("parse task gate pipeline");
    let default_input = asset
        .spec
        .default_input
        .as_ref()
        .expect("task gate pipeline default input");
    let ttl_seconds = default_input["ttl_seconds"]
        .as_u64()
        .expect("numeric ttl_seconds");
    let dispatch_timeout_seconds = default_input["dispatch_timeout_seconds"]
        .as_u64()
        .expect("numeric dispatch_timeout_seconds");

    assert!(
        ttl_seconds >= dispatch_timeout_seconds,
        "reservation TTL must cover the child dispatch wait budget"
    );
}

/// [ORB-10129] Structural invariants of the triage pipeline: it is
/// single-flight (`max_active_runs: 1` — one half of the overlap
/// guarantee, the routine's `overlap: forbid` is the other), an empty
/// candidate list skips both downstream steps (clean no-op), and the
/// lifecycle write is the deterministic `apply_dispositions` step, not
/// the agent.
#[test]
fn triage_pipeline_is_single_flight_and_gates_on_candidates() {
    let yaml = DEFAULT_JOB_FILES
        .iter()
        .find_map(|(name, yaml)| (*name == "task_triage_pipeline").then_some(*yaml))
        .expect("task triage pipeline default exists");
    let asset = load_job_asset(yaml).expect("parse task triage pipeline");
    assert_eq!(asset.spec.max_active_runs, 1);

    let step_ids = asset
        .spec
        .steps
        .iter()
        .map(|step| step.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        step_ids,
        ["list_candidates", "triage", "apply_dispositions"]
    );

    for step_id in ["triage", "apply_dispositions"] {
        let step = asset
            .spec
            .steps
            .iter()
            .find(|step| step.id == step_id)
            .expect("triage pipeline step");
        assert_eq!(
            step.when.as_deref(),
            Some("{{ steps.list_candidates.output.candidate_count }} != 0"),
            "step {step_id} must be skipped on an empty candidate list"
        );
    }

    let apply = asset
        .spec
        .steps
        .iter()
        .find(|step| step.id == "apply_dispositions")
        .expect("apply step");
    match &apply.body {
        JobV2StepBody::TargetRef(target) => {
            assert_eq!(target.target, "activity:apply_triage_dispositions");
            let input = target.default_input.as_ref().expect("apply input");
            assert_eq!(
                input["dispositions"],
                Value::String("{{ steps.triage.output.dispositions }}".to_string())
            );
            assert_eq!(
                input["candidates"],
                Value::String("{{ steps.list_candidates.output.candidates }}".to_string())
            );
        }
        other => panic!("expected apply target ref, got {other:?}"),
    }
}

#[test]
fn workspace_ship_pipeline_resolves_then_waits_for_normal_auto_ship() {
    let yaml = DEFAULT_JOB_FILES
        .iter()
        .find_map(|(name, yaml)| (*name == "workspace_ship_pipeline").then_some(*yaml))
        .expect("workspace ship pipeline default exists");
    let asset = load_job_asset(yaml).expect("parse workspace ship pipeline");
    assert_eq!(asset.spec.max_active_runs, 1);
    assert_eq!(asset.spec.steps.len(), 3);
    assert_eq!(asset.spec.steps[0].id, "resolve_ship_input");
    assert_eq!(asset.spec.steps[1].id, "ship");
    assert_eq!(asset.spec.steps[2].id, "require_ship_success");

    match &asset.spec.steps[0].body {
        JobV2StepBody::TargetRef(target) => {
            assert_eq!(target.target, "activity:resolve_workspace_ship_input");
        }
        other => panic!("expected resolver target ref, got {other:?}"),
    }
    match &asset.spec.steps[1].body {
        JobV2StepBody::TargetRef(target) => {
            assert_eq!(target.target, "activity:invoke_and_wait");
            let input = target.default_input.as_ref().expect("ship input");
            assert_eq!(input["job_name"], "task_auto_pipeline");
            assert_eq!(
                input["run_input"],
                Value::String("{{ steps.resolve_ship_input.output }}".to_string())
            );
            assert!(input.get("task_ids").is_none());
        }
        other => panic!("expected invoke-and-wait target ref, got {other:?}"),
    }
    match &asset.spec.steps[2].body {
        JobV2StepBody::TargetRef(target) => {
            assert_eq!(target.target, "activity:pipeline_success_guard");
        }
        other => panic!("expected success guard target ref, got {other:?}"),
    }
    assert!(!yaml.contains("auto_ship"));
    assert!(!yaml.contains("ship-sweep"));
    assert!(!yaml.contains("type: shell"));
}

#[test]
fn default_jobs_template_only_declared_agent_loop_handoffs() {
    let agent_activity_names = DEFAULT_ACTIVITY_FILES
        .iter()
        .filter_map(|(name, yaml)| {
            let asset = load_activity_asset(yaml).ok()?;
            matches!(asset.spec.spec, ActivityV2Spec::AgentLoop(_)).then_some(*name)
        })
        .collect::<BTreeSet<_>>();
    let allowed_handoffs = BTreeSet::from([
        // [ORB-10129] The triage agent's dispositions flow into the
        // deterministic `apply_triage_dispositions` step, which bounds
        // them (candidates-only, environmental-only re-backlog, durable
        // budget) instead of trusting them.
        (
            "task_triage_pipeline",
            "triage",
            "steps.triage.output.dispositions",
        ),
    ]);

    for (job_name, yaml) in DEFAULT_JOB_FILES {
        let asset = load_job_asset(yaml)
            .unwrap_or_else(|err| panic!("default job {job_name} should parse: {err}"));
        let mut agent_step_ids = BTreeSet::new();
        for step in &asset.spec.steps {
            collect_agent_loop_step_ids(step, &agent_activity_names, &mut agent_step_ids);
        }

        if agent_step_ids.is_empty() {
            continue;
        }

        let mut template_strings = Vec::new();
        for step in &asset.spec.steps {
            collect_template_strings(step, &mut template_strings);
        }

        for agent_step_id in agent_step_ids {
            let forbidden = format!("steps.{agent_step_id}.output");
            for template in &template_strings {
                let allowed =
                    allowed_handoffs
                        .iter()
                        .any(|(allowed_job, allowed_step, allowed_path)| {
                            *allowed_job == *job_name
                                && *allowed_step == agent_step_id
                                && template.contains(allowed_path)
                        });
                assert!(
                    !template.contains(&forbidden) || allowed,
                    "default job {job_name} templates from agent_loop output: {template}"
                );
            }
        }
    }
}

#[test]
fn default_job_conditions_keep_comparisons_outside_template_tokens() {
    for (name, yaml) in DEFAULT_JOB_FILES {
        let asset = load_job_asset(yaml).unwrap_or_else(|err| {
            panic!("default job {name} should parse before condition checks: {err}")
        });
        for step in &asset.spec.steps {
            assert_step_condition_tokens_are_paths(step);
        }
    }
}

#[test]
fn task_shipment_jobs_resolve_default_recovery_activity() {
    let catalog = default_activity_catalog();

    for job_name in ["task_local_pipeline", "task_pr_pipeline"] {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == job_name).then_some(*yaml))
            .unwrap_or_else(|| panic!("default job {job_name} exists"));
        let mut asset = load_job_asset(yaml)
            .unwrap_or_else(|err| panic!("default job {job_name} should parse: {err}"));

        assert_eq!(asset.spec.recovery_activity.as_deref(), None);
        resolve_job_target_refs(&mut asset.spec, &catalog)
            .unwrap_or_else(|err| panic!("default job {job_name} refs resolve: {err}"));
        if job_name == "task_pr_pipeline" {
            assert_eq!(
                asset.spec.failure_activity.as_deref(),
                Some("pr_failure_handoff")
            );
            assert!(
                asset.spec.resolved_failure_activity.is_some(),
                "task PR terminal failure handoff must resolve from the shipped catalog"
            );
        } else {
            assert_eq!(asset.spec.failure_activity, None);
        }
        let recovery_steps = step_recovery_activities(&asset.spec);
        assert!(
            !recovery_steps.is_empty(),
            "default job {job_name} should wire recovery on direct shipment steps"
        );
        for (step_id, recovery_activity, resolved) in recovery_steps {
            assert_eq!(
                recovery_activity.as_deref(),
                Some("step_failure_recovery"),
                "step {step_id} should use default recovery activity"
            );
            assert!(
                resolved,
                "step {step_id} should cache its recovery activity"
            );
        }
    }
}

#[test]
fn orchestration_jobs_do_not_enable_generic_recovery() {
    for job_name in [
        "task_auto_pipeline",
        "task_gate_pipeline",
        "task_triage_pipeline",
        "workspace_ship_pipeline",
    ] {
        let yaml = DEFAULT_JOB_FILES
            .iter()
            .find_map(|(name, yaml)| (*name == job_name).then_some(*yaml))
            .unwrap_or_else(|| panic!("default job {job_name} exists"));
        let asset = load_job_asset(yaml)
            .unwrap_or_else(|err| panic!("default job {job_name} should parse: {err}"));

        assert_eq!(
            asset.spec.recovery_activity, None,
            "default job {job_name} should not generically recover child orchestration"
        );
    }
}

fn collect_agent_loop_step_ids<'a>(
    step: &'a JobV2Step,
    agent_activity_names: &BTreeSet<&str>,
    out: &mut BTreeSet<&'a str>,
) {
    match &step.body {
        JobV2StepBody::TargetRef(target) => {
            if let Some(activity_name) = target.target.strip_prefix("activity:")
                && agent_activity_names.contains(activity_name)
            {
                out.insert(step.id.as_str());
            }
        }
        JobV2StepBody::Target(target) => {
            if matches!(target.spec, ActivityV2Spec::AgentLoop(_)) {
                out.insert(step.id.as_str());
            }
        }
        JobV2StepBody::Parallel { parallel } => {
            for child in &parallel.branches {
                collect_agent_loop_step_ids(child, agent_activity_names, out);
            }
        }
        JobV2StepBody::FanOut { fan_out, .. } => {
            collect_agent_loop_step_ids(&fan_out.worker, agent_activity_names, out);
        }
        JobV2StepBody::Loop { loop_ } => {
            for child in &loop_.steps {
                collect_agent_loop_step_ids(child, agent_activity_names, out);
            }
        }
    }
}

fn step_recovery_activities(job: &JobV2) -> Vec<(&str, &Option<String>, bool)> {
    let mut out = Vec::new();
    for step in &job.steps {
        collect_step_recovery_activities(step, &mut out);
    }
    out
}

fn collect_step_recovery_activities<'a>(
    step: &'a JobV2Step,
    out: &mut Vec<(&'a str, &'a Option<String>, bool)>,
) {
    if step.recovery_activity.is_some() {
        out.push((
            step.id.as_str(),
            &step.recovery_activity,
            step.resolved_recovery_activity.is_some(),
        ));
    }
    match &step.body {
        JobV2StepBody::Parallel { parallel } => {
            for child in &parallel.branches {
                collect_step_recovery_activities(child, out);
            }
        }
        JobV2StepBody::FanOut { fan_out, .. } => {
            collect_step_recovery_activities(&fan_out.worker, out);
        }
        JobV2StepBody::Loop { loop_ } => {
            for child in &loop_.steps {
                collect_step_recovery_activities(child, out);
            }
        }
        JobV2StepBody::TargetRef(_) | JobV2StepBody::Target(_) => {}
    }
}

fn collect_template_strings<'a>(step: &'a JobV2Step, out: &mut Vec<&'a str>) {
    if let Some(when) = &step.when {
        out.push(when);
    }

    match &step.body {
        JobV2StepBody::TargetRef(target) => {
            collect_value_strings(target.default_input.as_ref(), out);
        }
        JobV2StepBody::Target(target) => {
            collect_value_strings(target.default_input.as_ref(), out);
        }
        JobV2StepBody::Parallel { parallel } => {
            for child in &parallel.branches {
                collect_template_strings(child, out);
            }
        }
        JobV2StepBody::FanOut { fan_out, .. } => {
            out.push(&fan_out.items);
            collect_template_strings(&fan_out.worker, out);
        }
        JobV2StepBody::Loop { loop_ } => {
            if let Some(items) = &loop_.items {
                out.push(items);
            }
            if let Some(break_when) = &loop_.break_when {
                out.push(break_when);
            }
            for child in &loop_.steps {
                collect_template_strings(child, out);
            }
        }
    }
}

fn collect_value_strings<'a>(value: Option<&'a Value>, out: &mut Vec<&'a str>) {
    match value {
        Some(Value::String(text)) => out.push(text),
        Some(Value::Array(items)) => {
            for item in items {
                collect_value_strings(Some(item), out);
            }
        }
        Some(Value::Object(map)) => {
            for item in map.values() {
                collect_value_strings(Some(item), out);
            }
        }
        _ => {}
    }
}

#[test]
fn workspace_job_overrides_global_default_in_catalog_listing() {
    let (_root, runtime, global_root, workspace_root) = test_runtime();
    let global_job = global_root.join("resources/jobs/task_auto_pipeline.yaml");
    let workspace_job = workspace_root.join("resources/jobs/task_auto_pipeline.yaml");
    write_job(&global_job, "task_auto_pipeline", "global_action", 1);
    write_job(&workspace_job, "task_auto_pipeline", "workspace_action", 7);

    let jobs = runtime
        .list_job_catalog_with_last_run(true, JobCatalogFilter::All)
        .expect("list job catalog");
    let matches = jobs
        .iter()
        .filter(|(entry, _)| entry.job_id == "task_auto_pipeline")
        .collect::<Vec<_>>();

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].0.path, workspace_job);
    assert_eq!(matches[0].0.spec.max_active_runs, 7);
}

#[test]
fn job_listing_prefers_workspace_over_global() {
    let (_root, runtime, global_root, workspace_root) = test_runtime();
    let workspace_dir = workspace_root.join("resources/jobs");
    let global_dir = global_root.join("resources/jobs");
    write_job(
        &workspace_dir.join("layered.yaml"),
        "layered",
        "workspace",
        7,
    );
    write_job(&global_dir.join("layered.yaml"), "layered", "global", 1);

    let entry = runtime
        .show_job_catalog_entry("layered")
        .expect("layered job");
    assert_eq!(entry.path, workspace_dir.join("layered.yaml"));
    assert_eq!(entry.spec.max_active_runs, 7);
}

#[test]
fn job_execution_prefers_global_over_workspace() {
    let (_root, runtime, global_root, workspace_root) = test_runtime();
    let workspace_dir = workspace_root.join("resources/jobs");
    let global_dir = global_root.join("resources/jobs");
    write_job(&workspace_dir.join("custom.yaml"), "custom", "workspace", 7);
    write_job(&global_dir.join("custom.yaml"), "custom", "global", 1);
    write_job(
        &workspace_dir.join("task_auto_pipeline.yaml"),
        "task_auto_pipeline",
        "workspace",
        7,
    );
    write_job(
        &global_dir.join("task_auto_pipeline.yaml"),
        "task_auto_pipeline",
        "global",
        1,
    );

    let custom = runtime
        .load_v2_job_asset_by_name("custom")
        .expect("load custom catalog");
    assert_eq!(custom.0, global_dir.join("custom.yaml"));

    let default = runtime
        .load_v2_job_asset_by_name("task_auto_pipeline")
        .expect("load default catalog");
    assert_eq!(default.0, global_dir.join("task_auto_pipeline.yaml"));
}

#[test]
fn workspace_job_overrides_global_default_in_catalog_lookup_but_not_execution_lookup() {
    let (_root, runtime, global_root, workspace_root) = test_runtime();
    let global_job = global_root.join("resources/jobs/task_auto_pipeline.yaml");
    let workspace_job = workspace_root.join("resources/jobs/task_auto_pipeline.yaml");
    write_job(&global_job, "task_auto_pipeline", "global_action", 1);
    write_job(&workspace_job, "task_auto_pipeline", "workspace_action", 7);

    let entry = runtime
        .show_job_catalog_entry("task_auto_pipeline")
        .expect("catalog entry");
    assert_eq!(entry.path, workspace_job);
    assert_eq!(entry.spec.max_active_runs, 7);

    let (path, spec) = runtime
        .load_v2_job_asset_by_name("task_auto_pipeline")
        .expect("job lookup");
    assert_eq!(path, global_job);
    assert_eq!(spec.max_active_runs, 1);
}

#[test]
fn duplicate_jobs_within_one_catalog_directory_remain_invalid() {
    let (_root, runtime, _global_root, workspace_root) = test_runtime();
    let jobs_dir = workspace_root.join("resources/jobs");
    write_job(&jobs_dir.join("first.yaml"), "duplicate_job", "first", 1);
    write_job(
        &jobs_dir.join("nested/second.yaml"),
        "duplicate_job",
        "second",
        1,
    );

    let err = runtime
        .show_job_catalog_entry("duplicate_job")
        .expect_err("duplicate job name should fail");
    assert!(
        err.to_string()
            .contains("duplicate v2 job name 'duplicate_job'"),
        "{err}"
    );
}

#[test]
fn malformed_job_assets_remain_hard_catalog_errors() {
    let (_root, runtime, _global_root, workspace_root) = test_runtime();
    let malformed = workspace_root.join("resources/jobs/malformed.yaml");
    std::fs::create_dir_all(malformed.parent().expect("job path has parent"))
        .expect("create jobs dir");
    std::fs::write(&malformed, "schemaVersion: 2\nkind: Job\nspec: [")
        .expect("write malformed job");

    let err = runtime
        .list_job_catalog_with_last_run(true, JobCatalogFilter::All)
        .expect_err("malformed job should fail catalog loading");
    assert!(err.to_string().contains("malformed.yaml"), "{err}");
    assert!(err.to_string().contains("parse"), "{err}");
}
