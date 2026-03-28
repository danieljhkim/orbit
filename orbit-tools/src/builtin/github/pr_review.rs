use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, Tool, ToolContext, check_exec_result, require_str};

pub struct GithubPrReviewTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let repo = require_str(input, "repo")?;
    let pr = super::require_pr(input)?;
    let action = require_str(input, "action")?;

    let body = input.get("body").and_then(Value::as_str);

    let event = match action.as_str() {
        "approve" => "APPROVE",
        "request-changes" => "REQUEST_CHANGES",
        other => {
            return Err(OrbitError::InvalidInput(format!(
                "invalid `action`: \"{other}\"; must be approve or request-changes"
            )));
        }
    };

    if action.as_str() == "request-changes" && body.is_none() {
        return Err(OrbitError::InvalidInput(format!(
            "`body` is required for action \"{action}\""
        )));
    }

    let review_body = if let Some(b) = body {
        super::append_signature(b, ctx, "Reviewed")
    } else {
        super::agent_signature(ctx, "Reviewed").unwrap_or_default()
    };

    // POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews
    let endpoint = format!("repos/{repo}/pulls/{pr}/reviews");

    let mut args = vec![
        "api".to_string(),
        endpoint,
        "-f".to_string(),
        format!("event={event}"),
    ];

    if !review_body.is_empty() {
        args.push("-f".to_string());
        args.push(format!("body={review_body}"));
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
            description: "Approve or request changes on a pull request review".to_string(),
            parameters: vec![
                ToolParam {
                    name: "repo".to_string(),
                    description: "Repository in owner/name format".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "pr".to_string(),
                    description: "PR number".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "action".to_string(),
                    description: "Review action: approve or request-changes".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "body".to_string(),
                    description: "Review body (required for request-changes action)".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        let result = run_process(&req, &NoSandbox)?;
        check_exec_result(&result, "gh api (pr review)")?;
        let response: Value = serde_json::from_str(result.stdout.trim()).unwrap_or(json!({}));
        let id = response.get("id").and_then(Value::as_u64).unwrap_or(0);
        Ok(json!({
            "id": id,
            "reviewed": true,
        }))
    }
}
