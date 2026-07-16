use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use serde_json::{Value, json};

use crate::builtin::git::require_workspace_repo_root;
use crate::{TIMEOUT_LONG_MS, Tool, ToolContext};

pub struct GitPushTool;

impl Tool for GitPushTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "git.push".to_string(),
            description: "Push a local branch to a remote".to_string(),
            parameters: vec![
                ToolParam {
                    name: "repo_root".to_string(),
                    description: "Absolute path to the git repository root".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "branch".to_string(),
                    description: "Local branch name to push".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "remote".to_string(),
                    description: "Remote name (default: origin)".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "force_with_lease".to_string(),
                    description: "If true, push with an exact expected-SHA force-with-lease"
                        .to_string(),
                    param_type: "boolean".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "expected_remote_sha".to_string(),
                    description: "Exact remote branch SHA required when force_with_lease is true"
                        .to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let repo_root = require_workspace_repo_root(ctx, &input)?;
        let branch = input
            .get("branch")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| OrbitError::InvalidInput("missing `branch`".to_string()))?;
        let remote = input
            .get("remote")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("origin");
        let force_with_lease = input
            .get("force_with_lease")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let expected_remote_sha = input
            .get("expected_remote_sha")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if remote.starts_with('-') {
            return Err(OrbitError::InvalidInput(
                "remote name must not start with '-'".to_string(),
            ));
        }
        if branch.starts_with('-') {
            return Err(OrbitError::InvalidInput(
                "branch name must not start with '-'".to_string(),
            ));
        }
        if force_with_lease && !is_valid_expected_remote_sha(expected_remote_sha) {
            return Err(OrbitError::InvalidInput(
                "force_with_lease requires an exact 40- or 64-character expected_remote_sha"
                    .to_string(),
            ));
        }

        let args = push_args(
            &repo_root,
            remote,
            branch,
            force_with_lease.then_some(expected_remote_sha).flatten(),
        );

        let result = run_process(
            &ExecRequest {
                program: "git".to_string(),
                args,
                current_dir: None,
                timeout_ms: Some(TIMEOUT_LONG_MS),
                stdin_mode: StdinMode::Null,
                environment_mode: EnvironmentMode::Inherit,
                debug: false,
            },
            &NoSandbox,
        )?;

        if !result.success {
            return Err(OrbitError::Execution(format!(
                "git push failed: {}",
                result.stderr.trim()
            )));
        }

        Ok(json!({
            "repo_root": repo_root.to_string_lossy(),
            "remote": remote,
            "branch": branch,
            "stdout": result.stdout,
            "stderr": result.stderr,
        }))
    }
}

pub(super) fn push_args(
    repo_root: &std::path::Path,
    remote: &str,
    branch: &str,
    expected_remote_sha: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "-C".to_string(),
        repo_root.to_string_lossy().to_string(),
        "push".to_string(),
    ];
    if let Some(expected_remote_sha) = expected_remote_sha {
        args.push(format!(
            "--force-with-lease=refs/heads/{branch}:{expected_remote_sha}"
        ));
    }
    args.push("--".to_string());
    args.push(remote.to_string());
    args.push(branch.to_string());
    args
}

pub(super) fn is_valid_expected_remote_sha(value: Option<&str>) -> bool {
    value.is_some_and(|sha| {
        matches!(sha.len(), 40 | 64) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}
