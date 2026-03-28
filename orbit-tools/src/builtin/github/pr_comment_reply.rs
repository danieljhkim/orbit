use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, Tool, ToolContext, check_exec_result, require_str};

pub struct GithubPrCommentReplyTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<(ExecRequest, String), OrbitError> {
    let repo = super::require_repo(input)?;
    let pr = super::require_pr(input)?;
    let comment_id = super::require_numeric_str(input, "comment_id")?;
    let body = require_str(input, "body")?;
    let body = super::append_signature(&body, ctx, "Reviewed");

    // gh api repos/{owner}/{repo}/pulls/{pr}/comments/{comment_id}/replies -f body=...
    let endpoint = format!("repos/{repo}/pulls/{pr}/comments/{comment_id}/replies");

    let args = vec![
        "api".to_string(),
        endpoint,
        "-f".to_string(),
        format!("body={body}"),
    ];

    Ok((
        ExecRequest {
            program: "gh".to_string(),
            args,
            current_dir: None,
            timeout_ms: Some(TIMEOUT_DEFAULT_MS),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        body,
    ))
}

impl Tool for GithubPrCommentReplyTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "github.pr.comment.reply".to_string(),
            description: "Reply to a pull request review comment thread".to_string(),
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
                    name: "comment_id".to_string(),
                    description: "ID of the review comment to reply to".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "body".to_string(),
                    description: "Reply text".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let (req, _body) = build_exec_request(ctx, &input)?;
        let result = run_process(&req, &NoSandbox)?;
        check_exec_result(&result, "gh api (pr comment reply)")?;
        let response: Value = serde_json::from_str(result.stdout.trim()).unwrap_or(json!({}));
        let id = response.get("id").and_then(Value::as_u64).unwrap_or(0);
        Ok(json!({
            "id": id,
            "replied": true,
        }))
    }
}
