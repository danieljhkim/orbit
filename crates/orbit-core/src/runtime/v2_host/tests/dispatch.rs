use super::super::dispatch::*;
use crate::OrbitRuntime;
use crate::command::task::{TaskAddParams, TaskUpdateParams};
use crate::{ShipMode, WorkspaceRuntimeBinding};
use chrono::Utc;
use orbit_common::types::activity_job::V2ActivityCatalog;
use orbit_common::types::{
    ActivityV2Spec, NO_DIFF_EXPECTED_TAG, PipelineState, TaskPriority, TaskStatus, TaskType,
};
use orbit_engine::DispatchError;
use orbit_engine::V2RuntimeHost;
use orbit_tools::ToolContext;
use serde_json::json;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn seed_task(
    runtime: &OrbitRuntime,
    title: &str,
    status: TaskStatus,
    dependencies: Vec<String>,
) -> String {
    runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            description: format!("Fixture task: {title}"),
            acceptance_criteria: vec!["Fixture task is observable.".to_string()],
            dependencies,
            plan: "Fixture plan.".to_string(),
            workspace_path: Some(".".to_string()),
            priority: TaskPriority::Medium,
            task_type: Some(TaskType::Chore),
            status: Some(status),
            ..Default::default()
        })
        .expect("seed task")
        .id
}

fn seed_pr_handoff_task(runtime: &OrbitRuntime, title: &str, tags: Vec<String>) -> String {
    let task = runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            description: format!("Fixture task: {title}"),
            acceptance_criteria: vec!["Fixture task is observable.".to_string()],
            tags,
            plan: "Fixture plan.".to_string(),
            workspace_path: Some(".".to_string()),
            priority: TaskPriority::Medium,
            task_type: Some(TaskType::Chore),
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        })
        .expect("seed PR handoff task");
    runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                execution_summary: Some(
                    "Outcome: success\nChanges:\n- Fixture ready for handoff.".to_string(),
                ),
                implemented_by: Some(Some("codex".to_string())),
                job_run_id: Some(Some("checkpoint-run".to_string())),
                ..Default::default()
            },
        )
        .expect("prepare PR handoff task");
    task.id
}

#[test]
fn checkpointed_pr_handoff_actions_are_dispatched_by_v2_host() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    // These inputs stop at action-specific validation, proving the v2 host
    // forwarded them to the engine instead of rejecting their names.
    for action in ["pr_prepare", "git_rebase"] {
        let err = runtime
            .run_deterministic(action, &json!({}), &json!({}), ToolContext::default())
            .expect_err("incomplete fixture input should fail inside the action");
        match err {
            DispatchError::DeterministicActionFailed {
                action: reported_action,
                ..
            } => assert_eq!(reported_action, action),
            other => panic!("expected registered action failure, got {other}"),
        }
    }

    let no_diff_task = seed_pr_handoff_task(
        &runtime,
        "No-diff promotion",
        vec![NO_DIFF_EXPECTED_TAG.to_string()],
    );
    let no_diff = runtime
        .run_deterministic(
            "pr_promote",
            &json!({}),
            &json!({
                "job_run_id": "checkpoint-run",
                "completed_task_ids": [no_diff_task.clone()],
                "workspace_path": ".",
                "no_diff_expected": true,
            }),
            ToolContext::default(),
        )
        .expect("promote no-diff handoff");
    assert_eq!(no_diff["decision"], json!("performed"));
    assert!(no_diff["pr_number"].is_null());
    let no_diff_task = runtime
        .get_task(&no_diff_task)
        .expect("promoted no-diff task");
    assert_eq!(no_diff_task.status, TaskStatus::Review);
    assert!(no_diff_task.github_pr_number().is_none());

    let diff_task = seed_pr_handoff_task(&runtime, "PR promotion", Vec::new());
    let diff = runtime
        .run_deterministic(
            "pr_promote",
            &json!({}),
            &json!({
                "job_run_id": "checkpoint-run",
                "completed_task_ids": [diff_task.clone()],
                "workspace_path": ".",
                "pr_number": "42",
                "pr_url": "https://example.test/pull/42",
            }),
            ToolContext::default(),
        )
        .expect("promote PR handoff");
    assert_eq!(diff["decision"], json!("performed"));
    assert_eq!(diff["pr_number"], json!("42"));
    let diff_task = runtime.get_task(&diff_task).expect("promoted PR task");
    assert_eq!(diff_task.status, TaskStatus::Review);
    assert_eq!(diff_task.github_pr_number(), Some("42"));
}

/// [ORB-10410] `worktree_gc` ships as a deterministic activity reached
/// through the seeded (deliberately disabled) `worktree_gc` routine, so
/// nothing exercised it until an operator opted in — and by then the v2
/// allowlist no longer carried its name. Invoking the action directly must
/// reach the engine's reaper and return its structured GC envelope.
#[test]
fn worktree_gc_is_dispatchable_directly_through_the_v2_host() {
    let (_root, runtime, repo_root) = super::super::test_support::runtime_with_workspace_layout();
    let git_init = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&repo_root)
        .output()
        .expect("run git init");
    assert!(
        git_init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&git_init.stderr)
    );

    let output = runtime
        .run_deterministic(
            "worktree_gc",
            &json!({}),
            &json!({ "older_than_hours": 24 }),
            ToolContext::default(),
        )
        .expect("worktree_gc must dispatch through the v2 host");

    // This workspace has recorded no job runs, so the reaper considers no
    // worktrees. The assertion is the envelope itself: only the real
    // action produces it, and an unregistered name never gets this far.
    assert_eq!(output["reports"], json!([]));
    assert_eq!(output["bytes_reclaimed"], json!(0));
    assert!(
        output["dry_run"].is_boolean(),
        "GC envelope reports its deletion mode: {output}"
    );
}

#[test]
fn run_planning_duel_is_registered_for_v2_deterministic_dispatch() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let err = runtime
        .run_deterministic(
            "run_planning_duel",
            &json!({}),
            &json!({}),
            ToolContext::default(),
        )
        .expect_err("empty input should fail validation inside the action");

    match err {
        DispatchError::DeterministicActionFailed { action, message } => {
            assert_eq!(action, "run_planning_duel");
            assert!(
                message.contains("missing required input.task_id"),
                "unexpected validation message: {message}"
            );
        }
        other => panic!("expected registered action failure, got {other}"),
    }
}

#[test]
fn workspace_ship_input_prefers_the_registry_neutral_runtime_binding() {
    let root = tempdir().expect("tempdir");
    let global = root.path().join("global");
    let repo = root.path().join("repo");
    let workspace = repo.join(".orbit");
    std::fs::create_dir_all(&global).expect("global orbit");
    std::fs::create_dir_all(&workspace).expect("workspace orbit");
    std::fs::write(
        workspace.join("config.toml"),
        "[workflow]\nbase_branch = \"agent-main\"\n",
    )
    .expect("workspace config");

    let runtime = OrbitRuntime::from_roots_with_binding(
        &global,
        &workspace,
        WorkspaceRuntimeBinding {
            workspace_id: "ws_bound".to_string(),
            repo_root: repo,
            ship_mode: ShipMode::Pr,
        },
    )
    .expect("bound runtime");
    let input = runtime
        .run_deterministic(
            "resolve_workspace_ship_input",
            &json!({}),
            &json!({}),
            ToolContext::default(),
        )
        .expect("resolve bound ship input without a workspace registry");

    assert_eq!(input, json!({"mode": "pr", "base_branch": "agent-main"}));
}

#[test]
fn promote_agent_main_stub_is_loudly_fenced() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let err = runtime
        .run_deterministic(
            "promote_agent_main",
            &json!({}),
            &json!({
                "source_branch": "agent-main",
                "target_branch": "main",
            }),
            ToolContext::default(),
        )
        .expect_err("retired promotion stub should fail loudly");

    match err {
        DispatchError::DeterministicActionFailed { action, message } => {
            assert_eq!(action, "promote_agent_main");
            assert!(message.contains("retired stub"), "{message}");
            assert!(message.contains("git_merge"), "{message}");
            assert!(message.contains("git_push"), "{message}");
        }
        other => panic!("expected registered action failure, got {other}"),
    }
}

#[test]
fn revert_on_red_stub_is_loudly_fenced() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let err = runtime
        .run_deterministic(
            "revert_on_red",
            &json!({}),
            &json!({
                "commit_sha": "abc123",
                "branch": "agent-main",
            }),
            ToolContext::default(),
        )
        .expect_err("retired revert stub should fail loudly");

    match err {
        DispatchError::DeterministicActionFailed { action, message } => {
            assert_eq!(action, "revert_on_red");
            assert!(message.contains("retired stub"), "{message}");
            assert!(message.contains("manual incident task"), "{message}");
        }
        other => panic!("expected registered action failure, got {other}"),
    }
}

#[test]
fn reserve_locks_records_unmet_dependencies_in_run_state() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let dependency = seed_task(&runtime, "Dependency", TaskStatus::Backlog, Vec::new());
    let blocked = seed_task(
        &runtime,
        "Blocked",
        TaskStatus::Backlog,
        vec![dependency.clone()],
    );
    let run = runtime
        .stores()
        .jobs()
        .insert_job_run("task_gate_pipeline", 1, Utc::now(), Some(json!({})), None)
        .expect("insert run");
    runtime
        .stores()
        .jobs()
        .write_run_state(
            &run.run_id,
            &PipelineState::new(run.run_id.clone(), run.job_id.clone(), json!({})),
        )
        .expect("write state");

    let output = runtime
        .run_deterministic(
            "reserve_locks",
            &json!({}),
            &json!({
                "run_id": run.run_id,
                "task_ids": [blocked],
            }),
            ToolContext::default(),
        )
        .expect("reserve locks");

    assert_eq!(output["reserved"], json!(false));
    assert_eq!(output["waiting_on_deps"], json!([dependency]));
    let state = runtime
        .read_run_state(&run.run_id)
        .expect("read run state")
        .expect("state exists");
    assert_eq!(state.waiting_on_deps, Some(vec![dependency]));
    assert_eq!(state.waiting_on_locks, None);
}

#[test]
fn waiting_locks_from_reserve_output_extracts_unique_conflict_files() {
    let locks = waiting_locks_from_reserve_output(&json!({
        "reserved": false,
        "conflicts": [
            { "file": "file:src/lib.rs", "held_by_id": "ORB-1" },
            { "file": "file:src/lib.rs", "held_by_id": "reservation-1" },
            { "file": "dir:crates/orbit-core/src", "held_by_id": "ORB-2" }
        ],
    }));

    assert_eq!(
        locks,
        vec![
            "dir:crates/orbit-core/src".to_string(),
            "file:src/lib.rs".to_string()
        ]
    );
}

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points at <repo>/crates/orbit-core. Walk up two
    // levels to reach the workspace root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("orbit-core has a parent (crates/)")
        .parent()
        .expect("crates/ has a parent (repo root)")
        .to_path_buf()
}

/// Loads every activity asset under `dir` through the same catalog loader
/// `OrbitRuntime::v2_activity_catalog` uses at runtime, and returns the
/// deterministic actions it names that this dispatcher does not register.
/// A missing `dir` is not itself a failure — callers assert on the
/// unregistered-actions list, so an empty (or absent) tree just yields none.
fn unregistered_deterministic_actions(dir: &Path) -> Vec<String> {
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut catalog = V2ActivityCatalog::new();
    catalog
        .load_dir_skipping_retired(dir)
        .unwrap_or_else(|error| panic!("load activity assets from {}: {error}", dir.display()));

    catalog
        .names()
        .filter_map(|name| {
            let ActivityV2Spec::Deterministic(spec) = &catalog.get(name).expect("just listed").spec
            else {
                return None;
            };
            (!is_deterministic_action_registered(&spec.action))
                .then(|| format!("{name} (action: {})", spec.action))
        })
        .collect()
}

/// [ORB-10415] Guardrail for orbit-engine cleanup audit §2/§11: a shipped
/// activity asset naming a deterministic action this dispatcher does not
/// register is exactly the live defect that silently broke the PR failure
/// handoff (`pr_failure_handoff`) and the worktree GC routine (`worktree_gc`)
/// until ORB-10410. Load through the real catalog loader (not grep) so any
/// future asset drift fails a test instead of a production run.
#[test]
fn shipped_activity_assets_only_name_registered_deterministic_actions() {
    let root = repo_root();
    let mut unregistered =
        unregistered_deterministic_actions(&root.join("crates/orbit-core/assets/activities"));
    unregistered.extend(unregistered_deterministic_actions(
        &root.join(".orbit/resources/activities"),
    ));

    assert!(
        unregistered.is_empty(),
        "shipped activity assets name deterministic actions missing from \
         REGISTERED_DETERMINISTIC_ACTIONS: {unregistered:?}"
    );
}
