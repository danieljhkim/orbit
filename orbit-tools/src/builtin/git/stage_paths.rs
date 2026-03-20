use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::builtin::git::{require_relative_file_paths, require_repo_root};
use crate::{Tool, ToolContext};

pub struct GitStagePathsTool;

impl Tool for GitStagePathsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "git.stage_paths".to_string(),
            description: "Stage explicit file paths inside a git repository".to_string(),
            parameters: vec![
                ToolParam {
                    name: "repo_root".to_string(),
                    description: "Absolute path to the git repository root".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "files".to_string(),
                    description: "Explicit file paths to stage".to_string(),
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

        let mut args = vec![
            "-C".to_string(),
            repo_root.to_string_lossy().to_string(),
            "add".to_string(),
            "--".to_string(),
        ];
        args.extend(files.iter().cloned());

        let result = run_process(
            &ExecRequest {
                program: "git".to_string(),
                args,
                current_dir: None,
                timeout_ms: Some(30_000),
                stdin_mode: StdinMode::Null,
                environment_mode: EnvironmentMode::Inherit,
            debug: false,
            },
            &NoSandbox,
        )?;

        if !result.success {
            return Err(OrbitError::Execution(format!(
                "git add failed: {}",
                result.stderr.trim()
            )));
        }

        Ok(json!({
            "repo_root": repo_root.to_string_lossy(),
            "staged_files": files,
        }))
    }
}
