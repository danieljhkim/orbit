use orbit_common::types::OrbitError;
use orbit_exec::ExecRequest;
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, check_exec_result, require_str};

pub(super) fn build_exec_request(
    ctx: &crate::ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
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

    Ok(super::gh_exec_request(args, None, TIMEOUT_DEFAULT_MS))
}

// pub(super) visibility widened from private so that github::tests::pr_comment_reply (sibling test after nested collapse)
// can invoke the helper. See ORB-00243 and docs/design-patterns/test_layout.md.
pub(super) fn parse_reply_response(stdout: &str) -> Result<Value, OrbitError> {
    let id = super::parse_gh_api_id(stdout, "gh api (pr comment reply)")?;
    Ok(json!({
        "id": id,
        "replied": true,
    }))
}

super::gh_tool! {
    pub struct GithubPrCommentReplyTool;
    name: "github.pr.comment.reply";
    description: "Reply to a pull request review comment thread";
    parameters: [
        super::tool_param("repo", "Repository in owner/name format", "string", true),
        super::tool_param("pr", "PR number", "string", true),
        super::tool_param("comment_id", "ID of the review comment to reply to", "string", true),
        super::tool_param("body", "Reply text", "string", true),
    ];
    request: |ctx, input| {
        build_exec_request(ctx, input)
    }
    response: |_ctx, _input, result| {
        check_exec_result(result, "gh api (pr comment reply)")?;
        parse_reply_response(&result.stdout)
    }
}

