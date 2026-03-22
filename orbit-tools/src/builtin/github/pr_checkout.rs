use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{TIMEOUT_LONG_MS, Tool, ToolContext, check_exec_result};

pub struct GithubPrCheckoutTool;

pub(super) fn build_exec_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    let pr = super::require_pr(input)?;

    Ok(ExecRequest {
        program: "gh".to_string(),
        args: vec!["pr".to_string(), "checkout".to_string(), pr],
        current_dir: None,
        timeout_ms: Some(TIMEOUT_LONG_MS),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    })
}

impl Tool for GithubPrCheckoutTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "github.pr.checkout".to_string(),
            description: "Check out a pull request branch locally".to_string(),
            parameters: vec![ToolParam {
                name: "pr".to_string(),
                description: "PR number, URL, or branch name".to_string(),
                param_type: "string".to_string(),
                required: true,
            }],
            builtin: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(&input)?;
        let result = run_process(&req, &NoSandbox)?;
        check_exec_result(&result, "gh pr checkout")?;
        Ok(json!({
            "stdout": result.stdout,
            "stderr": result.stderr,
        }))
    }
}
