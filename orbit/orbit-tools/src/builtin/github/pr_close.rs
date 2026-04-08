use orbit_exec::ExecRequest;
use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, check_exec_result};

pub(super) fn build_exec_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    let pr = super::require_pr(input)?;

    let mut args = vec!["pr".to_string(), "close".to_string(), pr];

    if let Some(repo) = input.get("repo").and_then(Value::as_str) {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }

    Ok(super::gh_exec_request(args, None, TIMEOUT_DEFAULT_MS))
}

super::gh_tool! {
    pub struct GithubPrCloseTool;
    name: "github.pr.close";
    description: "Close a pull request without merging";
    parameters: [
        super::tool_param("pr", "PR number, URL, or branch name", "string", true),
        super::tool_param("repo", "Repository in owner/name format", "string", false),
    ];
    request: |_ctx, input| {
        build_exec_request(input)
    }
    response: |_ctx, _input, result| {
        check_exec_result(result, "gh pr close")?;
        Ok(json!({
            "stdout": result.stdout.as_str(),
            "stderr": result.stderr.as_str(),
        }))
    }
}
