use std::path::Path;

use orbit_exec::ExecRequest;
use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::builtin::fs::check_workspace_boundary;
use crate::{TIMEOUT_SLOW_MS, check_exec_result, require_str};

pub(super) fn build_exec_request(
    ctx: &crate::ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let title = require_str(input, "title")?;
    let base = require_str(input, "base")?;
    let head = require_str(input, "head")?;

    let body = input.get("body").and_then(Value::as_str);
    let body_file = input.get("body_file").and_then(Value::as_str);

    if body.is_none() && body_file.is_none() {
        return Err(OrbitError::InvalidInput(
            "one of `body` or `body_file` is required".to_string(),
        ));
    }

    let mut args = vec![
        "pr".to_string(),
        "create".to_string(),
        "--title".to_string(),
        title.to_string(),
        "--base".to_string(),
        base.to_string(),
        "--head".to_string(),
        head.to_string(),
    ];

    if let Some(b) = body {
        args.push("--body".to_string());
        args.push(super::append_signature(b, ctx, "Implemented"));
    } else if let Some(f) = body_file {
        let validated = check_workspace_boundary(ctx, Path::new(f))?;
        args.push("--body-file".to_string());
        args.push(validated.to_string_lossy().to_string());
    }

    if let Some(label) = input.get("label").and_then(Value::as_str) {
        args.push("--label".to_string());
        args.push(label.to_string());
    }

    if let Some(repo) = input.get("repo").and_then(Value::as_str) {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }

    Ok(super::gh_exec_request(
        args,
        ctx.cwd.clone(),
        TIMEOUT_SLOW_MS,
    ))
}

super::gh_tool! {
    pub struct GithubPrCreateTool;
    name: "github.pr.create";
    description: "Create a pull request";
    parameters: [
        super::tool_param("title", "Pull request title", "string", true),
        super::tool_param("base", "Base branch to merge into", "string", true),
        super::tool_param("head", "Head branch containing the changes", "string", true),
        super::tool_param(
            "body",
            "Pull request body text (required if body_file is absent)",
            "string",
            false,
        ),
        super::tool_param(
            "body_file",
            "Path to a file containing the PR body (required if body is absent)",
            "string",
            false,
        ),
        super::tool_param(
            "label",
            "Label to apply (optional, omitted if not provided)",
            "string",
            false,
        ),
        super::tool_param("repo", "Repository in owner/name format", "string", false),
    ];
    request: |ctx, input| {
        build_exec_request(ctx, input)
    }
    response: |_ctx, _input, result| {
        check_exec_result(result, "gh pr create")?;
        Ok(json!({
            "url": result.stdout.trim(),
            "stdout": result.stdout.as_str(),
            "stderr": result.stderr.as_str(),
        }))
    }
}
