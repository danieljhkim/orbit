use orbit_exec::ExecRequest;
use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::{TIMEOUT_LONG_MS, check_exec_result};

pub(super) fn build_exec_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    Ok(super::gh_exec_request(
        vec![
            "pr".to_string(),
            "checkout".to_string(),
            super::require_pr(input)?,
        ],
        None,
        TIMEOUT_LONG_MS,
    ))
}

super::gh_tool! {
    pub struct GithubPrCheckoutTool;
    name: "github.pr.checkout";
    description: "Check out a pull request branch locally";
    parameters: [
        super::tool_param("pr", "PR number, URL, or branch name", "string", true),
    ];
    request: |_ctx, input| {
        build_exec_request(input)
    }
    response: |_ctx, _input, result| {
        check_exec_result(result, "gh pr checkout")?;
        Ok(json!({
            "stdout": result.stdout.as_str(),
            "stderr": result.stderr.as_str(),
        }))
    }
}
