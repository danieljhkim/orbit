use orbit_exec::{ExecRequest, NoSandbox, run_process};
use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, check_exec_result};

pub(super) fn build_exec_requests(
    ctx: &crate::ToolContext,
    input: &Value,
) -> Result<(ExecRequest, ExecRequest), OrbitError> {
    let pr = super::require_pr(input)?;
    let repo_root = input
        .get("repo")
        .and_then(Value::as_str)
        .map(|repo| format!("repos/{repo}"))
        .unwrap_or_else(|| "repos/{owner}/{repo}".to_string());

    let review_req = gh_api_request(ctx, format!("{repo_root}/pulls/{pr}/comments"));
    let issue_req = gh_api_request(ctx, format!("{repo_root}/issues/{pr}/comments"));

    Ok((review_req, issue_req))
}

fn gh_api_request(ctx: &crate::ToolContext, endpoint: String) -> ExecRequest {
    super::gh_exec_request(
        vec![
            "api".to_string(),
            endpoint,
            "--paginate".to_string(),
            "--slurp".to_string(),
        ],
        ctx.cwd.clone(),
        TIMEOUT_DEFAULT_MS,
    )
}

fn parse_comment_pages(stdout: &str, label: &str) -> Result<Vec<Value>, OrbitError> {
    let payload: Value = serde_json::from_str(stdout).map_err(|error| {
        OrbitError::Execution(format!("failed to parse gh api {label} output: {error}"))
    })?;

    match payload {
        Value::Array(items) => {
            let mut comments = Vec::new();
            for item in items {
                match item {
                    Value::Array(page) => comments.extend(page),
                    Value::Object(_) => comments.push(item),
                    other => {
                        return Err(OrbitError::Execution(format!(
                            "gh api {label} returned unexpected item type: {}",
                            json_type_name(&other)
                        )));
                    }
                }
            }
            Ok(comments)
        }
        other => Err(OrbitError::Execution(format!(
            "gh api {label} returned unexpected payload type: {}",
            json_type_name(&other)
        ))),
    }
}

fn normalize_review_comment(comment: &Value) -> Value {
    json!({
        "id": comment.get("id").cloned().unwrap_or(Value::Null),
        "author": comment
            .get("user")
            .and_then(|value| value.get("login"))
            .and_then(Value::as_str),
        "body": comment.get("body").and_then(Value::as_str),
        "created_at": comment.get("created_at").and_then(Value::as_str),
        "in_reply_to_id": comment.get("in_reply_to_id").cloned().unwrap_or(Value::Null),
        "path": comment.get("path").cloned().unwrap_or(Value::Null),
        "line": comment.get("line").cloned().unwrap_or(Value::Null),
    })
}

fn normalize_issue_comment(comment: &Value) -> Value {
    json!({
        "id": comment.get("id").cloned().unwrap_or(Value::Null),
        "author": comment
            .get("user")
            .and_then(|value| value.get("login"))
            .and_then(Value::as_str),
        "body": comment.get("body").and_then(Value::as_str),
        "created_at": comment.get("created_at").and_then(Value::as_str),
        "in_reply_to_id": Value::Null,
        "path": Value::Null,
        "line": Value::Null,
    })
}

fn merge_comments(review_comments: Vec<Value>, issue_comments: Vec<Value>) -> Vec<Value> {
    let mut comments = review_comments
        .into_iter()
        .map(|comment| normalize_review_comment(&comment))
        .chain(
            issue_comments
                .into_iter()
                .map(|comment| normalize_issue_comment(&comment)),
        )
        .collect::<Vec<_>>();

    comments.sort_by(|left, right| {
        let left_ts = left
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let right_ts = right
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or_default();
        left_ts.cmp(right_ts)
    });
    comments
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

super::gh_tool! {
    pub struct GithubPrCommentsTool;
    name: "github.pr.comments";
    description: "List both general pull request comments and inline review comments";
    parameters: [
        super::tool_param("pr", "PR number", "string", true),
        super::tool_param("repo", "Repository in owner/name format", "string", false),
    ];
    execute: |ctx, input| {
        let (review_req, issue_req) = build_exec_requests(ctx, &input)?;

        let review_result = run_process(&review_req, &NoSandbox)?;
        check_exec_result(&review_result, "gh api (pr review comments)")?;
        let review_comments =
            parse_comment_pages(&review_result.stdout, "pull request review comments")?;

        let issue_result = run_process(&issue_req, &NoSandbox)?;
        check_exec_result(&issue_result, "gh api (pr issue comments)")?;
        let issue_comments =
            parse_comment_pages(&issue_result.stdout, "pull request issue comments")?;

        Ok(json!({
            "comments": merge_comments(review_comments, issue_comments),
        }))
    }
}
