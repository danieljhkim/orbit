use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::builtin::git::require_repo_root;
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
                    description:
                        "If true, push with --force-with-lease (safer force push that refuses to clobber unexpected remote changes)"
                            .to_string(),
                    param_type: "boolean".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let repo_root = require_repo_root(&input)?;
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

        let mut args = vec![
            "-C".to_string(),
            repo_root.to_string_lossy().to_string(),
            "push".to_string(),
        ];
        if force_with_lease {
            args.push("--force-with-lease".to_string());
        }
        args.push("--".to_string());
        args.push(remote.to_string());
        args.push(branch.to_string());

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
