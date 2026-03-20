use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct GithubPrCloseTool;

pub(super) fn build_exec_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    let pr = super::require_pr(input)?;

    let mut args = vec!["pr".to_string(), "close".to_string(), pr];

    if let Some(repo) = input.get("repo").and_then(Value::as_str) {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }

    Ok(ExecRequest {
        program: "gh".to_string(),
        args,
        current_dir: None,
        timeout_ms: Some(15_000),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    })
}

impl Tool for GithubPrCloseTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "github.pr.close".to_string(),
            description: "Close a pull request without merging".to_string(),
            parameters: vec![
                ToolParam {
                    name: "pr".to_string(),
                    description: "PR number, URL, or branch name".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "repo".to_string(),
                    description: "Repository in owner/name format".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(&input)?;
        let result = run_process(&req, &NoSandbox)?;

        if !result.success {
            return Err(OrbitError::Execution(format!(
                "gh pr close failed: {}",
                result.stderr.trim()
            )));
        }

        Ok(json!({
            "stdout": result.stdout,
            "stderr": result.stderr,
        }))
    }
}
