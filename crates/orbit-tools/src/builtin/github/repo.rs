use orbit_common::OrbitError;
use orbit_exec::ExecRequest;
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, check_exec_result};

pub fn build_exec_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    let mut args = vec!["repo".to_string(), "view".to_string()];

    if let Some(repo) = input.get("repo").and_then(Value::as_str) {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }

    args.push("--json".to_string());
    args.push("name,nameWithOwner,defaultBranchRef".to_string());

    Ok(super::gh_exec_request(args, None, TIMEOUT_DEFAULT_MS))
}

/// Reshape `gh repo view --json` into this surface's own field names.
///
/// `default_branch` is the repository's *release* branch as GitHub itself
/// reports it. Deriving it here is the reason nothing downstream has to guess
/// a branch name from convention.
pub fn project_repo_view(parsed: &Value) -> Value {
    json!({
        "name": parsed["name"],
        "full_name": parsed["nameWithOwner"],
        "default_branch": parsed["defaultBranchRef"]["name"],
    })
}

super::gh_tool! {
    pub struct GithubRepoViewTool;
    name: "github.repo.view";
    description: "Retrieve repository metadata including name and default branch";
    parameters: [
        super::tool_param(
            "repo",
            "Repository in owner/name format (uses current directory if omitted)",
            "string",
            false,
        ),
    ];
    request: |_ctx, input| {
        build_exec_request(input)
    }
    response: |_ctx, _input, result| {
        check_exec_result(result, "gh repo view")?;

        let parsed: Value = serde_json::from_str(&result.stdout).map_err(|e| {
            OrbitError::Execution(format!("failed to parse gh repo view output: {e}"))
        })?;

        Ok(project_repo_view(&parsed))
    }
}
