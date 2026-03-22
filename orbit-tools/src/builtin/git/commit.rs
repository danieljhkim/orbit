use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::builtin::git::{require_relative_file_paths, require_repo_root};
use crate::{TIMEOUT_SLOW_MS, Tool, ToolContext};

pub struct GitCommitTool;

impl Tool for GitCommitTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "git.commit".to_string(),
            description: "Create a git commit for an explicit file list".to_string(),
            parameters: vec![
                ToolParam {
                    name: "repo_root".to_string(),
                    description: "Absolute path to the git repository root".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "message".to_string(),
                    description: "Commit message to use".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "files".to_string(),
                    description: "Explicit file paths to include in the commit".to_string(),
                    param_type: "array".to_string(),
                    required: true,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let repo_root = require_repo_root(&input)?;
        let files = require_relative_file_paths(&input, &repo_root)?;
        let message = input
            .get("message")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| OrbitError::InvalidInput("missing `message`".to_string()))?;

        let mut args = vec![
            "-C".to_string(),
            repo_root.to_string_lossy().to_string(),
            "commit".to_string(),
            "-m".to_string(),
            message.to_string(),
            "--only".to_string(),
            "--".to_string(),
        ];
        args.extend(files.iter().cloned());

        let result = run_process(
            &ExecRequest {
                program: "git".to_string(),
                args,
                current_dir: None,
                timeout_ms: Some(TIMEOUT_SLOW_MS),
                stdin_mode: StdinMode::Null,
                environment_mode: EnvironmentMode::Inherit,
                debug: false,
            },
            &NoSandbox,
        )?;

        if !result.success {
            return Err(OrbitError::Execution(format!(
                "git commit failed: {}",
                result.stderr.trim()
            )));
        }

        Ok(json!({
            "repo_root": repo_root.to_string_lossy(),
            "message": message,
            "committed_files": files,
            "stdout": result.stdout,
            "stderr": result.stderr,
        }))
    }
}
