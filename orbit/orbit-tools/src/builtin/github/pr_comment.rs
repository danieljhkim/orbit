use orbit_exec::ExecRequest;
use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, check_exec_result, require_str};

pub(super) fn build_exec_request(
    ctx: &crate::ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let pr = super::require_pr(input)?;
    let body = require_str(input, "body")?;

    let mut args = vec![
        "pr".to_string(),
        "comment".to_string(),
        pr,
        "--body".to_string(),
        super::append_signature(&body, ctx, "Reviewed"),
    ];

    if let Some(repo) = input.get("repo").and_then(Value::as_str) {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }

    Ok(super::gh_exec_request(args, None, TIMEOUT_DEFAULT_MS))
}

super::gh_tool! {
    pub struct GithubPrCommentTool;
    name: "github.pr.comment";
    description: "Post a comment on a pull request";
    parameters: [
        super::tool_param("pr", "PR number, URL, or branch name", "string", true),
        super::tool_param("body", "Comment text", "string", true),
        super::tool_param("repo", "Repository in owner/name format", "string", false),
    ];
    request: |ctx, input| {
        build_exec_request(ctx, input)
    }
    response: |_ctx, _input, result| {
        check_exec_result(result, "gh pr comment")?;
        Ok(json!({
            "stdout": result.stdout.as_str(),
            "stderr": result.stderr.as_str(),
        }))
    }
}
