use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, Tool, ToolContext, check_exec_result, require_str};

pub struct GithubPrReviewTool;

pub(super) fn build_exec_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    let pr = super::require_pr(input)?;
    let action = require_str(input, "action")?;

    let body = input.get("body").and_then(Value::as_str);

    let action_flag = match action.as_str() {
        "approve" => "--approve",
        "request-changes" => "--request-changes",
        "comment" => "--comment",
        other => {
            return Err(OrbitError::InvalidInput(format!(
                "invalid `action`: \"{other}\"; must be approve, request-changes, or comment"
            )));
        }
    };

    if matches!(action.as_str(), "request-changes" | "comment") && body.is_none() {
        return Err(OrbitError::InvalidInput(format!(
            "`body` is required for action \"{action}\""
        )));
    }

    let mut args = vec![
        "pr".to_string(),
        "review".to_string(),
        pr,
        action_flag.to_string(),
    ];

    if let Some(b) = body {
        args.push("--body".to_string());
        args.push(b.to_string());
    }

    if let Some(repo) = input.get("repo").and_then(Value::as_str) {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }

    Ok(ExecRequest {
        program: "gh".to_string(),
        args,
        current_dir: None,
        timeout_ms: Some(TIMEOUT_DEFAULT_MS),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    })
}

impl Tool for GithubPrReviewTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "github.pr.review".to_string(),
            description: "Approve, request changes, or comment on a pull request review"
                .to_string(),
            parameters: vec![
                ToolParam {
                    name: "pr".to_string(),
                    description: "PR number, URL, or branch name".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "action".to_string(),
                    description: "Review action: approve, request-changes, or comment".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "body".to_string(),
                    description: "Review body (required for request-changes and comment actions)"
                        .to_string(),
                    param_type: "string".to_string(),
                    required: false,
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
        check_exec_result(&result, "gh pr review")?;
        Ok(json!({
            "stdout": result.stdout,
            "stderr": result.stderr,
        }))
    }
}
