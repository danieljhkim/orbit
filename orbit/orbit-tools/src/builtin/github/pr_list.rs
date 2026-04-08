use orbit_exec::ExecRequest;
use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, check_exec_result};

pub(super) fn build_exec_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    let state = input.get("state").and_then(Value::as_str).unwrap_or("open");

    let mut args = vec![
        "pr".to_string(),
        "list".to_string(),
        "--state".to_string(),
        state.to_string(),
        "--json".to_string(),
        "number,title,headRefName,author".to_string(),
    ];

    if let Some(label) = input.get("label").and_then(Value::as_str) {
        args.push("--label".to_string());
        args.push(label.to_string());
    }

    if let Some(repo) = input.get("repo").and_then(Value::as_str) {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }

    Ok(super::gh_exec_request(args, None, TIMEOUT_DEFAULT_MS))
}

super::gh_tool! {
    pub struct GithubPrListTool;
    name: "github.pr.list";
    description: "List pull requests, optionally filtered by label and state";
    parameters: [
        super::tool_param("label", "Filter by label (e.g. \"orbit\")", "string", false),
        super::tool_param(
            "state",
            "PR state filter: open (default), closed, or merged",
            "string",
            false,
        ),
        super::tool_param("repo", "Repository in owner/name format", "string", false),
    ];
    request: |_ctx, input| {
        build_exec_request(input)
    }
    response: |_ctx, _input, result| {
        check_exec_result(result, "gh pr list")?;

        let prs: Value = serde_json::from_str(&result.stdout).map_err(|e| {
            OrbitError::Execution(format!("failed to parse gh pr list output: {e}"))
        })?;

        Ok(json!({ "pull_requests": prs }))
    }
}
