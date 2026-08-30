use std::time::Duration;

use orbit_types::tool::ToolSessionContext;
use serde_json::json;

use crate::adapter::tool_host::HubCoordinationExecutor;

fn executor() -> (
    tempfile::TempDir,
    HubCoordinationExecutor,
    ToolSessionContext,
) {
    let root = tempfile::tempdir().expect("global root");
    HubCoordinationExecutor::register_workspace(root.path(), "ws_checkoutless", "checkoutless")
        .expect("register workspace");
    let executor = HubCoordinationExecutor::new(root.path(), "ws_checkoutless", None)
        .expect("coordination executor");
    let context = ToolSessionContext::trusted_local(
        Some("ws_checkoutless".to_string()),
        Some("hm_hub".to_string()),
        Some("hub".to_string()),
    );
    (root, executor, context)
}

/// Locate a task's bundle directory under a checkoutless hub root. The store
/// owns the layout; the test only needs *a* path to contend on.
fn task_bundle_dir(root: &std::path::Path, id: &str) -> std::path::PathBuf {
    fn walk(dir: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.file_name().is_some_and(|name| name == id) && path.join("task.yaml").is_file() {
                return Some(path);
            }
            if let Some(found) = walk(&path, id) {
                return Some(found);
            }
        }
        None
    }
    walk(root, id).unwrap_or_else(|| panic!("no bundle directory for {id} under {root:?}"))
}

/// ORB-11092: the checkoutless update path must hold the task lock across its
/// whole read-modify-write, not just around the store write.
///
/// The body reads the task, decides from that snapshot whether `required_tools`
/// may still change, and only then writes. When the lock covered the write
/// alone, a concurrent `orbit.task.start` could commit `in-progress` inside
/// that gap: the requirements updater had already observed a backlog task, and
/// its write landed a new allowlist after the lifecycle freeze.
///
/// The other thread holds the bundle lock directly, so the window is opened
/// deliberately rather than raced for. Sleep after the contender is announced
/// gives a stale pre-lock read time to observe backlog before start commits.
#[test]
fn checkoutless_concurrent_required_tools_update_cannot_write_through_start() {
    use std::sync::mpsc::sync_channel;

    let (root, executor, context) = executor();
    let created = executor
        .execute_tool(
            "orbit.task.add",
            json!({
                "workspace": "ws_checkoutless",
                "title": "Freeze under contention",
                "description": "required_tools must not race start",
                "complexity": "low",
                "required_tools": ["github.run.list"],
                "model": "codex"
            }),
            context.clone(),
        )
        .expect("add checkoutless task");
    let id = created["id"].as_str().expect("task id").to_string();
    executor
        .execute_tool(
            "orbit.task.update",
            json!({
                "id": id,
                "plan": "1. Execute with the admitted tool surface.",
                "model": "codex"
            }),
            context.clone(),
        )
        .expect("persist plan so start is legal");

    let lock_target = task_bundle_dir(root.path(), &id).join("task.yaml");
    let (locked_tx, locked_rx) = sync_channel::<()>(0);
    let (contender_tx, contender_rx) = sync_channel::<()>(0);

    let holder_executor = &executor;
    let holder_context = &context;
    let holder_lock_target = lock_target.as_path();
    let holder_id = id.as_str();
    let contended = std::thread::scope(|scope| {
        scope.spawn(move || {
            orbit_common::fs::io::with_exclusive_file_lock::<(), orbit_common::OrbitError, _>(
                holder_lock_target,
                "ORB-11092 regression",
                || {
                    locked_tx.send(()).expect("announce the held lock");
                    contender_rx.recv().expect("await the contending update");
                    std::thread::sleep(Duration::from_millis(250));
                    holder_executor
                        .execute_tool(
                            "orbit.task.start",
                            json!({"id": holder_id, "model": "codex"}),
                            holder_context.clone(),
                        )
                        .expect("start under the lock");
                    Ok(())
                },
            )
            .expect("hold the task lock");
        });

        locked_rx.recv().expect("await the held lock");
        contender_tx
            .send(())
            .expect("announce the contending update");
        executor.execute_tool(
            "orbit.task.update",
            json!({
                "id": id,
                "required_tools": ["github.run.view"],
                "model": "codex"
            }),
            context.clone(),
        )
    });

    let err = contended.expect_err("an in-progress task must refuse a required_tools change");
    assert!(
        err.to_string().contains("frozen"),
        "expected the required_tools freeze, got: {err}"
    );
    let shown = executor
        .execute_tool("orbit.task.show", json!({"id": id}), context)
        .expect("show after the race");
    assert_eq!(shown["status"], "in-progress");
    assert_eq!(
        shown["required_tools"],
        json!(["github.run.list"]),
        "the losing writer must not have changed required_tools after start"
    );
}

#[test]
fn checkoutless_required_tools_freeze_after_start_without_a_race() {
    let (_root, executor, context) = executor();
    let created = executor
        .execute_tool(
            "orbit.task.add",
            json!({
                "workspace": "ws_checkoutless",
                "title": "Hub required_tools freeze",
                "description": "Sequential freeze on the checkoutless path.",
                "complexity": "low",
                "required_tools": ["github.run.list", "github.auth.status"],
                "model": "codex"
            }),
            context.clone(),
        )
        .expect("add checkoutless task");
    let id = created["id"].as_str().expect("task id");
    executor
        .execute_tool(
            "orbit.task.update",
            json!({
                "id": id,
                "plan": "Execute with the admitted tool surface.",
                "model": "codex"
            }),
            context.clone(),
        )
        .expect("persist plan");
    let started = executor
        .execute_tool(
            "orbit.task.start",
            json!({"id": id, "model": "codex"}),
            context.clone(),
        )
        .expect("start checkoutless task");
    assert_eq!(started["status"], "in-progress");
    assert_eq!(
        started["required_tools"],
        json!(["github.auth.status", "github.run.list"])
    );

    let error = executor
        .execute_tool(
            "orbit.task.update",
            json!({"id": id, "required_tools": ["github.run.view"], "model": "codex"}),
            context,
        )
        .expect_err("active task requirements are frozen");
    assert!(error.to_string().contains("frozen"), "{error}");
}
