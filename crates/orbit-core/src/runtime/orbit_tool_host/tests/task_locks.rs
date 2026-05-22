use orbit_common::types::{OrbitError, TaskStatus};
use orbit_store::{TaskLockConflict, TaskLockHolder};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::OrbitRuntime;

use super::super::task_locks::{
    TaskLockReservationScope, parse_task_lock_reservation_scope, requested_task_files,
    task_lock_conflicts,
};
use super::super::test_support::{
    create_context_task, invalid_input_message, test_runtime, unmanaged_tool_env_guard,
};

fn v2_test_runtime() -> (TempDir, OrbitRuntime, std::path::PathBuf) {
    let root = tempfile::tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build v2 runtime");
    (root, runtime, repo_root)
}

#[test]
fn parse_task_lock_reservation_scope_requires_exactly_one_shape() {
    let _env = unmanaged_tool_env_guard();
    let missing = invalid_input_message(parse_task_lock_reservation_scope(&json!({})));
    assert!(missing.contains("exactly one of 'task_ids' or 'files' must be provided"));

    let both = invalid_input_message(parse_task_lock_reservation_scope(&json!({
        "task_ids": ["T20260506-15"],
        "files": ["file:src/lib.rs"],
    })));
    assert!(both.contains("exactly one of 'task_ids' or 'files' must be provided"));
}

#[test]
fn parse_task_lock_reservation_scope_validates_file_selectors() {
    let _env = unmanaged_tool_env_guard();
    let scope = parse_task_lock_reservation_scope(&json!({
        "files": ["file:src/../src/lib.rs", "dir:src/auth/"],
    }))
    .expect("parse files shape");
    assert_eq!(
        scope,
        TaskLockReservationScope::Files(vec![
            "dir:src/auth".to_string(),
            "file:src/lib.rs".to_string(),
        ])
    );

    let raw_path = invalid_input_message(parse_task_lock_reservation_scope(&json!({
        "files": ["src/lib.rs"],
    })));
    assert!(raw_path.contains("`file:`"));
    assert!(raw_path.contains("`dir:`"));

    let symbol = invalid_input_message(parse_task_lock_reservation_scope(&json!({
        "files": ["symbol:src/lib.rs#run:function"],
    })));
    assert!(symbol.contains("`file:`"));
    assert!(symbol.contains("`dir:`"));
    assert!(symbol.contains("`symbol:` selectors are not supported"));
}

#[test]
fn task_locks_reserve_adapter_surfaces_new_validation_errors() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, _repo_root) = test_runtime();

    let missing = invalid_input_message(runtime.execute_tool_command(
        "orbit.task.locks.reserve",
        json!({ "model": "gpt-5.5" }),
        None,
        None,
    ));
    assert!(missing.contains("exactly one of 'task_ids' or 'files' must be provided"));

    let both = invalid_input_message(runtime.execute_tool_command(
        "orbit.task.locks.reserve",
        json!({
            "task_ids": ["T20260506-15"],
            "files": ["file:src/lib.rs"],
            "model": "gpt-5.5",
        }),
        None,
        None,
    ));
    assert!(both.contains("exactly one of 'task_ids' or 'files' must be provided"));

    let raw_path = invalid_input_message(runtime.execute_tool_command(
        "orbit.task.locks.reserve",
        json!({
            "files": ["src/lib.rs"],
            "model": "gpt-5.5",
        }),
        None,
        None,
    ));
    assert!(raw_path.contains("`file:`"));
    assert!(raw_path.contains("`dir:`"));

    let symbol = invalid_input_message(runtime.execute_tool_command(
        "orbit.task.locks.reserve",
        json!({
            "files": ["symbol:src/lib.rs#run:function"],
            "model": "gpt-5.5",
        }),
        None,
        None,
    ));
    assert!(symbol.contains("`file:`"));
    assert!(symbol.contains("`dir:`"));
    assert!(symbol.contains("`symbol:` selectors are not supported"));
}

#[test]
fn requested_task_files_prune_missing_context_entries() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    std::fs::create_dir_all(repo_root.join("docs/design")).expect("create docs dir");
    std::fs::write(repo_root.join("docs/design/groundhog.md"), "alias").expect("write alias doc");

    let task = create_context_task(
        &runtime,
        &repo_root,
        TaskStatus::Backlog,
        &["docs/design/groundhog.md", "docs/design/missing.md"],
    );

    let requested =
        requested_task_files(&runtime, &[task.id]).expect("collect requested task files");
    assert_eq!(requested, vec!["file:docs/design/groundhog.md".to_string()]);
}

#[test]
fn task_lock_conflicts_ignore_missing_held_context_entries() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    std::fs::create_dir_all(repo_root.join("src")).expect("create src dir");
    std::fs::write(repo_root.join("src/lib.rs"), "pub fn ok() {}\n").expect("write source file");

    let holder = create_context_task(
        &runtime,
        &repo_root,
        TaskStatus::InProgress,
        &["docs/design/groundhog.md", "src/lib.rs"],
    );

    let conflicts = task_lock_conflicts(
        &runtime,
        &[],
        &[
            "docs/design/groundhog.md".to_string(),
            "src/lib.rs".to_string(),
        ],
    )
    .expect("compute task lock conflicts");

    assert_eq!(
        conflicts,
        vec![TaskLockConflict {
            file: "src/lib.rs".to_string(),
            held_by: TaskLockHolder::Task,
            held_by_id: holder.id,
        }]
    );
}

#[test]
fn task_lock_conflicts_use_selector_anchor_overlap() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    std::fs::create_dir_all(repo_root.join("src")).expect("create src dir");
    std::fs::write(repo_root.join("src/lib.rs"), "pub fn ok() {}\n").expect("write source file");

    let holder = create_context_task(
        &runtime,
        &repo_root,
        TaskStatus::InProgress,
        &["symbol:src/lib.rs#ok:function"],
    );

    let conflicts = task_lock_conflicts(
        &runtime,
        &[],
        &["file:src/lib.rs".to_string(), "dir:src".to_string()],
    )
    .expect("compute selector-aware task lock conflicts");

    assert_eq!(
        conflicts,
        vec![
            TaskLockConflict {
                file: "dir:src".to_string(),
                held_by: TaskLockHolder::Task,
                held_by_id: holder.id.clone(),
            },
            TaskLockConflict {
                file: "file:src/lib.rs".to_string(),
                held_by: TaskLockHolder::Task,
                held_by_id: holder.id,
            },
        ]
    );
}

#[test]
fn reservation_conflicts_clear_immediately_after_release() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    std::fs::create_dir_all(repo_root.join("src")).expect("create src dir");
    std::fs::write(repo_root.join("src/lib.rs"), "pub fn ok() {}\n").expect("write source file");

    let first = create_context_task(
        &runtime,
        &repo_root,
        TaskStatus::Backlog,
        &["file:src/lib.rs"],
    );
    let second = create_context_task(
        &runtime,
        &repo_root,
        TaskStatus::Backlog,
        &["file:src/lib.rs"],
    );

    let first_reserve = runtime
        .execute_tool_command(
            "orbit.task.locks.reserve",
            json!({
                "task_ids": [first.id.clone()],
                "ttl_seconds": 3600,
                "model": "gpt-5.5",
            }),
            None,
            None,
        )
        .expect("reserve first task");
    let reservation_id = first_reserve
        .get("reservation_id")
        .and_then(Value::as_str)
        .expect("reservation id is present")
        .to_string();

    let locks = runtime
        .execute_tool_command("orbit.task.locks", json!({}), None, None)
        .expect("list locks");
    assert_eq!(locks["total_reservations"], 1);
    assert_eq!(
        locks["by_reservation"][0]["reservation_id"],
        reservation_id.as_str()
    );
    assert_eq!(locks["by_reservation"][0]["task_ids"], json!([first.id]));
    assert_eq!(
        locks["by_reservation"][0]["files"],
        json!(["file:src/lib.rs"])
    );
    assert!(
        locks["by_reservation"][0]["expires_at"].is_string(),
        "reservation visibility should include expiration"
    );

    let blocked = runtime
        .execute_tool_command(
            "orbit.task.locks.reserve",
            json!({
                "task_ids": [second.id.clone()],
                "ttl_seconds": 3600,
                "model": "gpt-5.5",
            }),
            None,
            None,
        )
        .expect("second reservation returns conflict");
    assert_eq!(blocked["reserved"], false);
    assert_eq!(
        blocked["conflicts"],
        json!([{
            "file": "file:src/lib.rs",
            "held_by": "reservation",
            "held_by_id": reservation_id.clone(),
        }])
    );

    let release = runtime
        .execute_tool_command(
            "orbit.task.locks.release",
            json!({
                "reservation_id": reservation_id,
                "model": "gpt-5.5",
            }),
            None,
            None,
        )
        .expect("release reservation");
    assert_eq!(release["released"], true);

    let second_reserve = runtime
        .execute_tool_command(
            "orbit.task.locks.reserve",
            json!({
                "task_ids": [second.id],
                "ttl_seconds": 3600,
                "model": "gpt-5.5",
            }),
            None,
            None,
        )
        .expect("second reservation succeeds after release");
    assert_eq!(second_reserve["reserved"], true);
}

#[test]
fn v2_task_locks_store_workspace_binding_id() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = v2_test_runtime();
    std::fs::create_dir_all(repo_root.join("src")).expect("create src dir");
    std::fs::write(repo_root.join("src/lib.rs"), "pub fn ok() {}\n").expect("write source file");

    let task = create_context_task(
        &runtime,
        &repo_root,
        TaskStatus::Backlog,
        &["file:src/lib.rs"],
    );
    assert_eq!(task.id, "ORB-00000");

    let reservation = runtime
        .execute_tool_command(
            "orbit.task.locks.reserve",
            json!({
                "task_ids": [task.id],
                "ttl_seconds": 3600,
                "model": "gpt-5.5",
            }),
            None,
            None,
        )
        .expect("reserve v2 task");
    assert_eq!(reservation["reserved"], true);

    let locks = runtime
        .execute_tool_command("orbit.task.locks", json!({}), None, None)
        .expect("list locks");
    let workspace_id = locks["by_reservation"][0]["workspace_id"]
        .as_str()
        .expect("reservation carries workspace_id");
    assert!(workspace_id.starts_with("repo-"), "{workspace_id}");
}

#[test]
fn v2_task_locks_fail_when_workspace_binding_config_disappears() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = v2_test_runtime();
    std::fs::create_dir_all(repo_root.join("src")).expect("create src dir");
    std::fs::write(repo_root.join("src/lib.rs"), "pub fn ok() {}\n").expect("write source file");
    std::fs::remove_file(repo_root.join(".orbit/config.yaml")).expect("remove workspace config");

    let err = runtime
        .execute_tool_command(
            "orbit.task.locks.reserve",
            json!({
                "files": ["file:src/lib.rs"],
                "ttl_seconds": 3600,
                "model": "gpt-5.5",
            }),
            None,
            None,
        )
        .expect_err("missing v2 binding config should fail");
    assert!(matches!(
        err,
        OrbitError::Store(message)
            if message.contains("task artifact workspace config is missing")
    ));
}

#[test]
fn files_shape_reservations_conflict_and_release_like_task_reservations() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    std::fs::create_dir_all(repo_root.join("src")).expect("create src dir");
    std::fs::write(repo_root.join("src/lib.rs"), "pub fn ok() {}\n").expect("write source file");

    let direct_reserve = runtime
        .execute_tool_command(
            "orbit.task.locks.reserve",
            json!({
                "files": ["file:src/lib.rs", "dir:src/auth/"],
                "ttl_seconds": 3600,
                "model": "gpt-5.5",
            }),
            None,
            None,
        )
        .expect("reserve direct file selectors");
    assert_eq!(direct_reserve["reserved"], true);
    assert_eq!(
        direct_reserve["reserved_files"],
        json!(["dir:src/auth", "file:src/lib.rs"])
    );
    let reservation_id = direct_reserve
        .get("reservation_id")
        .and_then(Value::as_str)
        .expect("reservation id is present")
        .to_string();

    let locks = runtime
        .execute_tool_command("orbit.task.locks", json!({}), None, None)
        .expect("list locks");
    assert_eq!(locks["total_reservations"], 1);
    assert_eq!(
        locks["by_reservation"][0]["reservation_id"],
        reservation_id.as_str()
    );
    assert_eq!(locks["by_reservation"][0]["task_ids"], json!([]));
    assert_eq!(
        locks["by_reservation"][0]["files"],
        json!(["dir:src/auth", "file:src/lib.rs"])
    );

    let task = create_context_task(
        &runtime,
        &repo_root,
        TaskStatus::Backlog,
        &["file:src/lib.rs"],
    );
    let blocked = runtime
        .execute_tool_command(
            "orbit.task.locks.reserve",
            json!({
                "task_ids": [task.id.clone()],
                "ttl_seconds": 3600,
                "model": "gpt-5.5",
            }),
            None,
            None,
        )
        .expect("task reservation returns conflict");
    assert_eq!(blocked["reserved"], false);
    assert_eq!(
        blocked["conflicts"],
        json!([{
            "file": "file:src/lib.rs",
            "held_by": "reservation",
            "held_by_id": reservation_id.clone(),
        }])
    );

    let release = runtime
        .execute_tool_command(
            "orbit.task.locks.release",
            json!({
                "reservation_id": reservation_id,
                "model": "gpt-5.5",
            }),
            None,
            None,
        )
        .expect("release direct reservation");
    assert_eq!(release["released"], true);

    let task_reserve = runtime
        .execute_tool_command(
            "orbit.task.locks.reserve",
            json!({
                "task_ids": [task.id],
                "ttl_seconds": 3600,
                "model": "gpt-5.5",
            }),
            None,
            None,
        )
        .expect("task reservation succeeds after release");
    assert_eq!(task_reserve["reserved"], true);
}
