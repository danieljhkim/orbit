use orbit_common::types::OrbitError;
use orbit_common::types::{ToolParam, ToolSchema};
use orbit_exec::{EnvironmentMode, ExecRequest, StdinMode};
use serde_json::Value;

use crate::{ToolContext, ToolRegistry, require_str};

pub(super) fn gh_exec_request(
    args: Vec<String>,
    current_dir: Option<String>,
    timeout_ms: u64,
) -> ExecRequest {
    ExecRequest {
        program: "gh".to_string(),
        args,
        current_dir,
        timeout_ms: Some(timeout_ms),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    }
}

pub(super) fn gh_schema(name: &str, description: &str, parameters: Vec<ToolParam>) -> ToolSchema {
    ToolSchema {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
        builtin: true,
    }
}

pub(super) fn tool_param(
    name: &str,
    description: &str,
    param_type: &str,
    required: bool,
) -> ToolParam {
    ToolParam {
        name: name.to_string(),
        description: description.to_string(),
        param_type: param_type.to_string(),
        required,
    }
}

macro_rules! gh_tool {
    (
        $vis:vis struct $name:ident;
        name: $tool_name:expr;
        description: $description:expr;
        parameters: [$($param:expr),* $(,)?];
        request: |$request_ctx:ident, $request_input:ident| $request:block
        response: |$response_ctx:ident, $response_input:ident, $result:ident| $response:block
    ) => {
        $vis struct $name;

        impl crate::Tool for $name {
            fn schema(&self) -> orbit_common::types::ToolSchema {
                super::gh_schema($tool_name, $description, vec![$($param),*])
            }

            fn execute(
                &self,
                ctx: &crate::ToolContext,
                input: serde_json::Value,
            ) -> Result<serde_json::Value, orbit_common::types::OrbitError> {
                let req = {
                    let $request_ctx = ctx;
                    let $request_input = &input;
                    $request
                }?;
                let exec_result = orbit_exec::run_process(&req, &orbit_exec::NoSandbox)?;
                let $response_ctx = ctx;
                let $response_input = &input;
                let $result = &exec_result;
                $response
            }
        }
    };
    (
        $vis:vis struct $name:ident;
        name: $tool_name:expr;
        description: $description:expr;
        parameters: [$($param:expr),* $(,)?];
        execute: |$execute_ctx:ident, $execute_input:ident| $execute:block
    ) => {
        $vis struct $name;

        impl crate::Tool for $name {
            fn schema(&self) -> orbit_common::types::ToolSchema {
                super::gh_schema($tool_name, $description, vec![$($param),*])
            }

            fn execute(
                &self,
                ctx: &crate::ToolContext,
                input: serde_json::Value,
            ) -> Result<serde_json::Value, orbit_common::types::OrbitError> {
                let $execute_ctx = ctx;
                let $execute_input = input;
                $execute
            }
        }
    };
}

pub(super) use gh_tool;

pub mod auth;
pub mod pr_checkout;
pub mod pr_checks;
pub mod pr_close;
pub mod pr_comment;
pub mod pr_comment_reply;
pub mod pr_comments;
pub mod pr_create;
pub mod pr_list;
pub mod pr_merge;
pub mod pr_review;
pub mod pr_review_comment;
pub mod pr_view;
pub mod repo;

#[cfg(test)]
mod tests;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(pr_create::GithubPrCreateTool);
    registry.register(pr_view::GithubPrViewTool);
    registry.register(pr_comment::GithubPrCommentTool);
    registry.register(pr_comment_reply::GithubPrCommentReplyTool);
    registry.register(pr_comments::GithubPrCommentsTool);
    registry.register(pr_review::GithubPrReviewTool);
    registry.register(pr_review_comment::GithubPrReviewCommentTool);
    registry.register(pr_merge::GithubPrMergeTool);
}

/// Validate that `repo` matches the `owner/name` format expected by the GitHub API.
///
/// Rejects values containing path traversal sequences, extra slashes, or characters
/// outside the set GitHub allows for owner and repository names.
pub(super) fn validate_repo(repo: &str) -> Result<(), OrbitError> {
    // GitHub owner: alphanumeric or hyphen (no leading hyphen, no consecutive hyphens in org names,
    // but we keep the regex simple — GitHub itself will reject truly invalid names).
    // Repo name: alphanumeric, hyphen, underscore, or dot.
    // Exactly one slash separating owner and name.
    let valid = repo.split('/').count() == 2 && {
        let mut parts = repo.split('/');
        let owner = parts.next().unwrap_or("");
        let name = parts.next().unwrap_or("");
        !owner.is_empty()
            && !name.is_empty()
            && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    };
    if !valid {
        return Err(OrbitError::InvalidInput(format!(
            "invalid `repo` format: \"{repo}\"; expected owner/name (e.g. octocat/hello-world)"
        )));
    }
    Ok(())
}

/// Extract and validate a `repo` field in `owner/name` format.
pub(super) fn require_repo(input: &Value) -> Result<String, OrbitError> {
    let repo = require_str(input, "repo")?;
    validate_repo(&repo)?;
    Ok(repo)
}

/// Extract a non-empty `pr` field from the tool input.
/// Accepts a numeric PR number or a GitHub PR URL (extracts the number from the path).
pub(super) fn require_pr(input: &Value) -> Result<String, OrbitError> {
    let pr = require_str(input, "pr")?;
    // Already numeric — use directly.
    if !pr.is_empty() && pr.chars().all(|c| c.is_ascii_digit()) {
        return Ok(pr);
    }
    // Try to extract PR number from a GitHub URL like
    // https://github.com/owner/repo/pull/123
    if pr.contains("github.com/")
        && pr.contains("/pull/")
        && let Some(num) = pr.rsplit('/').next()
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
    {
        return Ok(num.to_string());
    }
    Err(OrbitError::InvalidInput(format!(
        "invalid `pr`: \"{pr}\"; must be a numeric PR number or GitHub PR URL"
    )))
}

/// Extract a non-empty numeric string field from tool input.
pub(super) fn require_numeric_str(input: &Value, key: &str) -> Result<String, OrbitError> {
    let value = require_str(input, key)?;
    if !value.chars().all(|c| c.is_ascii_digit()) || value.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "invalid `{key}`: \"{value}\"; must be numeric"
        )));
    }
    Ok(value)
}

pub(super) fn parse_gh_api_id(stdout: &str, label: &str) -> Result<u64, OrbitError> {
    let response: Value = serde_json::from_str(stdout.trim())
        .map_err(|err| OrbitError::Execution(format!("{label} returned invalid JSON: {err}")))?;
    response
        .as_object()
        .and_then(|object| object.get("id"))
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            OrbitError::Execution(format!(
                "{label} returned unexpected JSON: expected object with numeric `id`"
            ))
        })
}

/// Build an agent attribution footer line.
///
/// Returns `None` when `agent_name` is not set on the context (i.e. the tool
/// was not called from an agent execution path).
pub(super) fn agent_signature(ctx: &ToolContext, verb: &str) -> Option<String> {
    let agent = ctx.agent_name.as_deref()?;
    let model = ctx.model_name.as_deref().unwrap_or("unknown");
    Some(format!("*{verb} by: {agent} / {model}*"))
}

/// Append an agent attribution footer to a body string, if identity is available.
pub(super) fn append_signature(body: &str, ctx: &ToolContext, verb: &str) -> String {
    match agent_signature(ctx, verb) {
        Some(sig) => format!("{body}\n\n{sig}"),
        None => body.to_string(),
    }
}
