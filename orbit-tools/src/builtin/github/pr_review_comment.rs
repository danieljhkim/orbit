use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, Tool, ToolContext, check_exec_result, require_str};

pub struct GithubPrReviewCommentTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let repo = super::require_repo(input)?;
    let pr = super::require_pr(input)?;
    let body = require_str(input, "body")?;
    let path = require_str(input, "path")?;
    let line = input
        .get("line")
        .and_then(|v| {
            v.as_u64()
                .map(|n| n.to_string())
                .or_else(|| v.as_str().map(String::from))
        })
        .filter(|s| !s.is_empty())
        .ok_or_else(|| OrbitError::InvalidInput("missing `line`".to_string()))?;

    let body = super::append_signature(&body, ctx, "Reviewed");

    // Resolve the latest commit on the PR so the review comment is anchored
    // to the current head. GitHub requires `commit_id` for pull review comments.
    let commit_id = match input.get("commit_id").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => resolve_pr_head_sha(&repo, &pr)?,
    };

    let endpoint = format!("repos/{repo}/pulls/{pr}/comments");

    let args = vec![
        "api".to_string(),
        endpoint,
        "-f".to_string(),
        format!("body={body}"),
        "-f".to_string(),
        format!("path={path}"),
        "-F".to_string(),
        format!("line={line}"),
        "-f".to_string(),
        format!("commit_id={commit_id}"),
        "-f".to_string(),
        "side=RIGHT".to_string(),
    ];

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

fn resolve_pr_head_sha(repo: &str, pr: &str) -> Result<String, OrbitError> {
    let req = ExecRequest {
        program: "gh".to_string(),
        args: vec![
            "pr".to_string(),
            "view".to_string(),
            pr.to_string(),
            "--repo".to_string(),
            repo.to_string(),
            "--json".to_string(),
            "headRefOid".to_string(),
            "--jq".to_string(),
            ".headRefOid".to_string(),
        ],
        current_dir: None,
        timeout_ms: Some(TIMEOUT_DEFAULT_MS),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    };
    let result = run_process(&req, &NoSandbox)?;
    let sha = result.stdout.trim().to_string();
    if sha.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "could not resolve head commit for PR {pr} in {repo}"
        )));
    }
    Ok(sha)
}

impl Tool for GithubPrReviewCommentTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "github.pr.review.comment".to_string(),
            description:
                "Post an inline review comment on a specific file and line of a pull request"
                    .to_string(),
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
                    name: "path".to_string(),
                    description: "File path relative to the repository root".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "line".to_string(),
                    description: "Line number in the diff to attach the comment to".to_string(),
                    param_type: "number".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "body".to_string(),
                    description: "Comment text".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "commit_id".to_string(),
                    description: "Optional commit SHA to anchor the comment (defaults to PR head)"
                        .to_string(),
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
        check_exec_result(&result, "gh api (pr review comment)")?;
        let response: Value = serde_json::from_str(result.stdout.trim()).unwrap_or(json!({}));
        let id = response.get("id").and_then(Value::as_u64).unwrap_or(0);
        Ok(json!({
            "id": id,
            "commented": true,
        }))
    }
}
