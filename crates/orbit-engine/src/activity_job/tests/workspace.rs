#![allow(missing_docs)]

use tempfile::tempdir;

use super::super::dispatcher::DispatchError;
use super::super::workspace::*;

#[test]
fn resolve_subprocess_cwd_prefers_input_over_task_over_tool_ctx() {
    let input_dir = tempdir().expect("input tempdir");
    let task_dir = tempdir().expect("task tempdir");
    let tool_dir = tempdir().expect("tool tempdir");

    let input = serde_json::json!({
        "workspace_path": input_dir.path().display().to_string()
    });
    let task_ctx = serde_json::json!({
        "workspace_path": task_dir.path().display().to_string()
    });
    let resolved = resolve_subprocess_cwd(&input, Some(&task_ctx), Some(tool_dir.path()))
        .expect("input cwd resolves");
    assert_eq!(
        resolved,
        Some(
            input_dir
                .path()
                .canonicalize()
                .expect("canonical input dir")
        )
    );

    let input = serde_json::json!({});
    let resolved = resolve_subprocess_cwd(&input, Some(&task_ctx), Some(tool_dir.path()))
        .expect("task cwd resolves");
    assert_eq!(
        resolved,
        Some(task_dir.path().canonicalize().expect("canonical task dir"))
    );

    // Absent key (direct, non-worktree run): fall back to the tool context's
    // workspace_root — the repo root is the correct cwd there.
    let resolved =
        resolve_subprocess_cwd(&input, None, Some(tool_dir.path())).expect("tool cwd resolves");
    assert_eq!(
        resolved,
        Some(tool_dir.path().canonicalize().expect("canonical tool dir"))
    );
}

#[test]
fn resolve_subprocess_cwd_fails_closed_on_declared_non_string_workspace_path() {
    // Regression (ORB-10134): a worktree pipeline step whose workspace_path
    // template rendered to a non-string, non-null value must fail closed, not
    // silently fall back to the tool context's workspace_root (the primary
    // checkout).
    let tool_dir = tempdir().expect("tool tempdir");

    for value in [
        serde_json::json!(42),
        serde_json::json!(true),
        serde_json::json!({ "nested": "object" }),
    ] {
        let input = serde_json::json!({ "workspace_path": value });
        let err = resolve_subprocess_cwd(&input, None, Some(tool_dir.path()))
            .expect_err("non-string workspace_path must fail closed");
        match err {
            DispatchError::CliInvocationFailed(message) => {
                assert!(
                    message.contains("non-string workspace_path"),
                    "message should flag the non-string value: {message}"
                );
            }
            other => panic!("expected CliInvocationFailed, got {other:?}"),
        }
    }

    // An empty-string render is likewise refused (fail closed, not fall back).
    let input = serde_json::json!({ "workspace_path": "   " });
    let err = resolve_subprocess_cwd(&input, None, Some(tool_dir.path()))
        .expect_err("empty workspace_path must fail closed");
    assert!(matches!(err, DispatchError::CliInvocationFailed(_)));
}

#[test]
fn resolve_subprocess_cwd_treats_null_workspace_path_as_absent() {
    // The agent envelope / task context serialize an undeclared workspace_path
    // as JSON null; that must be treated as "not declared" so direct
    // (non-worktree) runs fall back to the tool context's workspace_root.
    let tool_dir = tempdir().expect("tool tempdir");

    let input = serde_json::json!({ "workspace_path": null });
    let resolved = resolve_subprocess_cwd(&input, None, Some(tool_dir.path()))
        .expect("null input workspace_path falls back to tool cwd");
    assert_eq!(
        resolved,
        Some(tool_dir.path().canonicalize().expect("canonical tool dir"))
    );

    // Same for a null workspace_path on the task context (its always-present
    // key), which is how a task with no declared workspace serializes.
    let task_ctx = serde_json::json!({ "workspace_path": null });
    let resolved = resolve_subprocess_cwd(
        &serde_json::json!({}),
        Some(&task_ctx),
        Some(tool_dir.path()),
    )
    .expect("null task-context workspace_path falls back to tool cwd");
    assert_eq!(
        resolved,
        Some(tool_dir.path().canonicalize().expect("canonical tool dir"))
    );
}

#[test]
fn resolve_subprocess_cwd_rejects_non_directory_path() {
    let temp = tempdir().expect("tempdir");
    let file = temp.path().join("not-a-dir");
    std::fs::write(&file, b"not a directory").expect("write file");
    let task_ctx = serde_json::json!({
        "workspace_path": file.display().to_string()
    });

    let err = resolve_subprocess_cwd(&serde_json::json!({}), Some(&task_ctx), None)
        .expect_err("file path rejected");
    match err {
        DispatchError::CliInvocationFailed(message) => {
            assert!(
                message.contains(&file.display().to_string()),
                "message should name file path: {message}"
            );
        }
        other => panic!("expected CliInvocationFailed, got {other:?}"),
    }
}

#[test]
fn resolve_subprocess_cwd_rejects_declared_missing_path() {
    let temp = tempdir().expect("tempdir");
    let missing = temp.path().join("missing-worktree");
    let input = serde_json::json!({
        "workspace_path": missing.display().to_string()
    });

    let err = resolve_subprocess_cwd(&input, None, None).expect_err("missing path rejected");
    match err {
        DispatchError::CliInvocationFailed(message) => {
            assert!(
                message.contains(&missing.display().to_string()),
                "message should name missing path: {message}"
            );
        }
        other => panic!("expected CliInvocationFailed, got {other:?}"),
    }
}
