use orbit_exec::ExecRequest;
use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, check_exec_result};

pub(super) fn build_exec_request(
    ctx: &crate::ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let pr = super::require_pr(input)?;

    let mut args = vec![
        "pr".to_string(),
        "view".to_string(),
        pr,
        "--json".to_string(),
        "number,title,body,headRefName,files,commits".to_string(),
    ];

    if let Some(repo) = input.get("repo").and_then(Value::as_str) {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }

    Ok(super::gh_exec_request(
        args,
        ctx.cwd.clone(),
        TIMEOUT_DEFAULT_MS,
    ))
}

super::gh_tool! {
    pub struct GithubPrViewTool;
    name: "github.pr.view";
    description: "Retrieve full metadata for a pull request";
    parameters: [
        super::tool_param("pr", "PR number or GitHub PR URL", "string", true),
        super::tool_param("repo", "Repository in owner/name format", "string", false),
    ];
    request: |ctx, input| {
        build_exec_request(ctx, input)
    }
    response: |_ctx, _input, result| {
        check_exec_result(result, "gh pr view")?;

        let pr: Value = serde_json::from_str(&result.stdout).map_err(|e| {
            OrbitError::Execution(format!("failed to parse gh pr view output: {e}"))
        })?;

        Ok(json!({ "pull_request": pr }))
    }
}
