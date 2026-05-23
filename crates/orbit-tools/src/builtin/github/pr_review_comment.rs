use orbit_common::types::OrbitError;
use orbit_exec::{ExecRequest, NoSandbox, run_process};
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, check_exec_result, require_str};

pub(super) fn build_exec_request(
    ctx: &crate::ToolContext,
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

    Ok(super::gh_exec_request(args, None, TIMEOUT_DEFAULT_MS))
}

fn resolve_pr_head_sha(repo: &str, pr: &str) -> Result<String, OrbitError> {
    let req = super::gh_exec_request(
        vec![
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
        None,
        TIMEOUT_DEFAULT_MS,
    );
    let result = run_process(&req, &NoSandbox)?;
    let sha = result.stdout.trim().to_string();
    if sha.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "could not resolve head commit for PR {pr} in {repo}"
        )));
    }
    Ok(sha)
}

// pub(super) visibility widened from private so that github::tests::pr_review_comment (sibling test after nested collapse)
// can invoke the helper. See ORB-00243 and docs/design-patterns/test_layout.md.
pub(super) fn parse_review_comment_response(stdout: &str) -> Result<Value, OrbitError> {
    let id = super::parse_gh_api_id(stdout, "gh api (pr review comment)")?;
    Ok(json!({
        "id": id,
        "commented": true,
    }))
}

super::gh_tool! {
    pub struct GithubPrReviewCommentTool;
    name: "github.pr.review.comment";
    description: "Post an inline review comment on a specific file and line of a pull request";
    parameters: [
        super::tool_param("repo", "Repository in owner/name format", "string", true),
        super::tool_param("pr", "PR number", "string", true),
        super::tool_param(
            "path",
            "File path relative to the repository root",
            "string",
            true,
        ),
        super::tool_param(
            "line",
            "Line number in the diff to attach the comment to",
            "number",
            true,
        ),
        super::tool_param("body", "Comment text", "string", true),
        super::tool_param(
            "commit_id",
            "Optional commit SHA to anchor the comment (defaults to PR head)",
            "string",
            false,
        ),
    ];
    request: |ctx, input| {
        build_exec_request(ctx, input)
    }
    response: |_ctx, _input, result| {
        check_exec_result(result, "gh api (pr review comment)")?;
        parse_review_comment_response(&result.stdout)
    }
}
