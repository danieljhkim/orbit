use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct GithubPrCommentTool;

pub(super) fn build_exec_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    let pr = super::require_pr(input)?;

    let body = input
        .get("body")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| OrbitError::InvalidInput("missing `body`".to_string()))?;

    let mut args = vec![
        "pr".to_string(),
        "comment".to_string(),
        pr,
        "--body".to_string(),
        body.to_string(),
    ];

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

impl Tool for GithubPrCommentTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "github.pr.comment".to_string(),
            description: "Post a comment on a pull request".to_string(),
            parameters: vec![
                ToolParam {
                    name: "pr".to_string(),
                    description: "PR number, URL, or branch name".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "body".to_string(),
                    description: "Comment text".to_string(),
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
                "gh pr comment failed: {}",
                result.stderr.trim()
            )));
        }

        Ok(json!({
            "stdout": result.stdout,
            "stderr": result.stderr,
        }))
    }
}
