use orbit_common::types::{OrbitError, TaskStatus};
use serde_json::{Value, json};

use super::super::test_support::{create_task, test_runtime, unmanaged_tool_env_guard};

#[test]
fn review_thread_add_rejects_missing_model() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Review thread missing model",
        "Exercise required-model enforcement.",
        TaskStatus::Review,
        &[],
    );

    let error = runtime
        .execute_tool_command(
            "orbit.task.review_thread.add",
            json!({
                "id": task.id,
                "body": "Review feedback.",
            }),
            None,
            None,
        )
        .expect_err("missing model should fail");

    assert!(matches!(error, OrbitError::InvalidInput(_)));
    assert!(
        error
            .to_string()
            .contains("orbit.task.review_thread.add requires `model`")
    );
}

#[test]
fn review_thread_add_rejects_empty_model() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Review thread empty model",
        "Exercise required-model enforcement on empty input.",
        TaskStatus::Review,
        &[],
    );

    let error = runtime
        .execute_tool_command(
            "orbit.task.review_thread.add",
            json!({
                "id": task.id,
                "body": "Review feedback.",
                "model": "   ",
            }),
            None,
            None,
        )
        .expect_err("empty model should fail");

    assert!(
        matches!(error, OrbitError::InvalidInput(_)),
        "expected InvalidInput, got: {error}"
    );
    assert!(
        error.to_string().contains("model"),
        "error should mention model: {error}"
    );
}

#[test]
fn review_thread_reply_rejects_missing_model() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Reply missing model",
        "Exercise required-model enforcement on reply.",
        TaskStatus::Review,
        &[],
    );

    let thread = runtime
        .add_review_thread(
            &task.id,
            "Initial review.".to_string(),
            None,
            None,
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("add review thread");

    let error = runtime
        .execute_tool_command(
            "orbit.task.review_thread.reply",
            json!({
                "id": task.id,
                "thread_id": thread.thread_id,
                "body": "Reply.",
            }),
            None,
            None,
        )
        .expect_err("missing model should fail");

    assert!(matches!(error, OrbitError::InvalidInput(_)));
    assert!(
        error
            .to_string()
            .contains("orbit.task.review_thread.reply requires `model`")
    );
}

#[test]
fn review_thread_add_accepts_human_model() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Human review accepted",
        "Exercise that explicit `model: human` opts out of scoring without erroring.",
        TaskStatus::Review,
        &[],
    );

    let output = runtime
        .execute_tool_command(
            "orbit.task.review_thread.add",
            json!({
                "id": task.id,
                "body": "Human review feedback.",
                "model": "human",
            }),
            None,
            None,
        )
        .expect("human-attributed review should succeed");

    assert_eq!(
        output.get("id").and_then(Value::as_str),
        Some(task.id.as_str())
    );
}

#[test]
fn review_thread_list_round_trips_task_level_anchor_kind() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Task-level review",
        "Exercise anchorless review thread output.",
        TaskStatus::Review,
        &[],
    );

    runtime
        .execute_tool_command(
            "orbit.task.review_thread.add",
            json!({
                "id": task.id,
                "body": "Task-level ask.",
                "model": "human",
            }),
            None,
            None,
        )
        .expect("add task-level thread");

    let output = runtime
        .execute_tool_command(
            "orbit.task.review_thread.list",
            json!({ "task_id": task.id }),
            None,
            None,
        )
        .expect("list review threads");
    assert_eq!(output[0]["anchor"]["kind"], "task_level");
    assert_eq!(output[0]["path"], Value::Null);
    assert_eq!(output[0]["line"], Value::Null);
}

#[test]
fn review_thread_add_uses_active_task_env_when_id_is_omitted() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    let task = create_task(
        &runtime,
        &repo_root,
        "Active task review",
        "Exercise active-task review-thread input.",
        TaskStatus::Review,
        &[],
    );
    // SAFETY: unmanaged_tool_env_guard serializes and restores this env mutation.
    unsafe {
        std::env::set_var("ORBIT_ACTIVE_TASK_ID", &task.id);
    }

    runtime
        .execute_tool_command(
            "orbit.task.review_thread.add",
            json!({
                "body": "Active task ask.",
                "model": "human",
            }),
            None,
            None,
        )
        .expect("active task add succeeds");

    let output = runtime
        .execute_tool_command(
            "orbit.task.review_thread.list",
            json!({ "task_id": task.id }),
            None,
            None,
        )
        .expect("list review threads");
    assert_eq!(output[0]["messages"][0]["body"], "Active task ask.");
}
