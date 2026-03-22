use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, Tool, ToolContext, check_exec_result};

pub struct GithubPrCommentsTool;

pub(super) fn build_exec_requests(
    ctx: &ToolContext,
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

fn gh_api_request(ctx: &ToolContext, endpoint: String) -> ExecRequest {
    ExecRequest {
        program: "gh".to_string(),
        args: vec![
            "api".to_string(),
            endpoint,
            "--paginate".to_string(),
            "--slurp".to_string(),
        ],
        current_dir: ctx.cwd.clone(),
        timeout_ms: Some(TIMEOUT_DEFAULT_MS),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    }
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

impl Tool for GithubPrCommentsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "github.pr.comments".to_string(),
            description: "List both general pull request comments and inline review comments"
                .to_string(),
            parameters: vec![
                ToolParam {
                    name: "pr".to_string(),
                    description: "PR number".to_string(),
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

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn build_exec_requests_uses_repo_placeholders_when_repo_absent() {
        let (review_req, issue_req) =
            build_exec_requests(&ToolContext::default(), &json!({ "pr": "42" })).expect("valid");

        assert_eq!(review_req.program, "gh");
        assert_eq!(review_req.args[0], "api");
        assert_eq!(review_req.args[1], "repos/{owner}/{repo}/pulls/42/comments");
        assert_eq!(issue_req.args[1], "repos/{owner}/{repo}/issues/42/comments");
    }

    #[test]
    fn build_exec_requests_uses_explicit_repo_when_provided() {
        let (review_req, issue_req) = build_exec_requests(
            &ToolContext {
                cwd: Some("/tmp/orbit".to_string()),
                ..Default::default()
            },
            &json!({
                "pr": "42",
                "repo": "owner/repo",
            }),
        )
        .expect("valid");

        assert_eq!(review_req.args[1], "repos/owner/repo/pulls/42/comments");
        assert_eq!(issue_req.args[1], "repos/owner/repo/issues/42/comments");
        assert_eq!(review_req.current_dir.as_deref(), Some("/tmp/orbit"));
    }

    #[test]
    fn parse_comment_pages_flattens_slurped_pages() {
        let comments = parse_comment_pages(
            r#"
            [
              [{"id": 1, "created_at": "2026-03-22T10:00:00Z"}],
              [{"id": 2, "created_at": "2026-03-22T11:00:00Z"}]
            ]
            "#,
            "review comments",
        )
        .expect("parse");

        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0]["id"], json!(1));
        assert_eq!(comments[1]["id"], json!(2));
    }

    #[test]
    fn merge_comments_normalizes_and_sorts_both_comment_types() {
        let merged = merge_comments(
            vec![json!({
                "id": 2,
                "user": { "login": "reviewer" },
                "body": "inline",
                "created_at": "2026-03-22T11:00:00Z",
                "in_reply_to_id": 10,
                "path": "src/lib.rs",
                "line": 12
            })],
            vec![json!({
                "id": 1,
                "user": { "login": "author" },
                "body": "general",
                "created_at": "2026-03-22T10:00:00Z"
            })],
        );

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0]["id"], json!(1));
        assert_eq!(merged[0]["author"], json!("author"));
        assert_eq!(merged[0]["path"], Value::Null);
        assert_eq!(merged[1]["id"], json!(2));
        assert_eq!(merged[1]["in_reply_to_id"], json!(10));
        assert_eq!(merged[1]["path"], json!("src/lib.rs"));
        assert_eq!(merged[1]["line"], json!(12));
    }
}
