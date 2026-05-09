use orbit_common::types::OrbitError;
use orbit_exec::ExecRequest;
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, check_exec_result, require_str};

pub(super) fn build_exec_request(
    ctx: &crate::ToolContext,
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

    Ok(super::gh_exec_request(args, None, TIMEOUT_DEFAULT_MS))
}

fn parse_review_response(stdout: &str) -> Result<Value, OrbitError> {
    let id = super::parse_gh_api_id(stdout, "gh api (pr review)")?;
    Ok(json!({
        "id": id,
        "reviewed": true,
    }))
}

super::gh_tool! {
    pub struct GithubPrReviewTool;
    name: "github.pr.review";
    description: "Approve or request changes on a pull request review";
    parameters: [
        super::tool_param("repo", "Repository in owner/name format", "string", true),
        super::tool_param("pr", "PR number", "string", true),
        super::tool_param(
            "action",
            "Review action: approve or request-changes",
            "string",
            true,
        ),
        super::tool_param(
            "body",
            "Review body (required for request-changes action)",
            "string",
            false,
        ),
    ];
    request: |ctx, input| {
        build_exec_request(ctx, input)
    }
    response: |_ctx, _input, result| {
        check_exec_result(result, "gh api (pr review)")?;
        parse_review_response(&result.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_review_response_returns_id_from_valid_stdout() {
        let response = parse_review_response(r#"{"id":12345,"state":"APPROVED"}"#).unwrap();

        assert_eq!(response["id"], json!(12345));
        assert_eq!(response["reviewed"], json!(true));
    }

    #[test]
    fn parse_review_response_rejects_malformed_stdout() {
        let error = parse_review_response("not json").unwrap_err();

        assert!(matches!(error, OrbitError::Execution(_)));
    }

    #[test]
    fn parse_review_response_rejects_empty_stdout() {
        let error = parse_review_response("").unwrap_err();

        assert!(matches!(error, OrbitError::Execution(_)));
    }

    #[test]
    fn parse_review_response_rejects_object_without_id() {
        let error = parse_review_response(r#"{"state":"APPROVED"}"#).unwrap_err();

        assert!(matches!(error, OrbitError::Execution(_)));
    }
}
