use tempfile::tempdir;

use super::super::*;

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

    let resolved =
        resolve_subprocess_cwd(&input, None, Some(tool_dir.path())).expect("tool cwd resolves");
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
