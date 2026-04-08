use orbit_exec::ExecRequest;
use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::TIMEOUT_DEFAULT_MS;

pub(super) fn build_exec_request(_input: &Value) -> Result<ExecRequest, OrbitError> {
    Ok(super::gh_exec_request(
        vec!["auth".to_string(), "status".to_string()],
        None,
        TIMEOUT_DEFAULT_MS,
    ))
}

super::gh_tool! {
    pub struct GithubAuthStatusTool;
    name: "github.auth.status";
    description: "Verify GitHub CLI authentication status";
    parameters: [];
    request: |_ctx, input| {
        build_exec_request(input)
    }
    response: |_ctx, _input, result| {
        Ok(json!({
            "authenticated": result.success,
            "stdout": result.stdout.as_str(),
            "stderr": result.stderr.as_str(),
        }))
    }
}
