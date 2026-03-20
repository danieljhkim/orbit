use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::builtin::git::require_repo_root;
use crate::{Tool, ToolContext};

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

        let result = run_process(
            &ExecRequest {
                program: "git".to_string(),
                args: vec![
                    "-C".to_string(),
                    repo_root.to_string_lossy().to_string(),
                    "push".to_string(),
                    remote.to_string(),
                    branch.to_string(),
                ],
                current_dir: None,
                timeout_ms: Some(60_000),
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
