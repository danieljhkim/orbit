use std::path::{Path, PathBuf};

use serde_json::Value;

use super::dispatcher::DispatchError;

pub fn resolve_subprocess_cwd(
    input: &Value,
    task_ctx: Option<&Value>,
    tool_ctx_workspace_root: Option<&Path>,
) -> Result<Option<PathBuf>, DispatchError> {
    if let Some(path) = input.get("workspace_path").and_then(Value::as_str) {
        return validate_declared_workspace_path(path);
    }

    if let Some(path) = task_ctx
        .and_then(|task| task.get("workspace_path"))
        .and_then(Value::as_str)
    {
        return validate_declared_workspace_path(path);
    }

    let Some(path) = tool_ctx_workspace_root else {
        return Ok(None);
    };

    if path.is_dir() {
        return Ok(Some(canonicalize_dir(path)));
    }

    tracing::warn!(
        target: "orbit.engine.cli_runner",
        path = %path.display(),
        "tool_ctx workspace_root missing, child will inherit parent cwd"
    );
    Ok(None)
}

fn validate_declared_workspace_path(path: &str) -> Result<Option<PathBuf>, DispatchError> {
    let path_buf = PathBuf::from(path);
    if path.trim().is_empty() || !path_buf.is_dir() {
        return Err(DispatchError::CliInvocationFailed(format!(
            "workspace path {} is not a writable directory",
            path_buf.display()
        )));
    }
    Ok(Some(canonicalize_dir(&path_buf)))
}

// pub(crate) widened for tests/ layout under ORB-00225; test reaches via exposed surface.
pub(crate) fn canonicalize_dir(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests;
