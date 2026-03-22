use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolSchema};
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, Tool, ToolContext};

pub struct GithubAuthStatusTool;

pub(super) fn build_exec_request(_input: &Value) -> Result<ExecRequest, OrbitError> {
    Ok(ExecRequest {
        program: "gh".to_string(),
        args: vec!["auth".to_string(), "status".to_string()],
        current_dir: None,
        timeout_ms: Some(TIMEOUT_DEFAULT_MS),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    })
}

impl Tool for GithubAuthStatusTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "github.auth.status".to_string(),
            description: "Verify GitHub CLI authentication status".to_string(),
            parameters: vec![],
            builtin: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(&input)?;
        let result = run_process(&req, &NoSandbox)?;
        Ok(json!({
            "authenticated": result.success,
            "stdout": result.stdout,
            "stderr": result.stderr,
        }))
    }
}
