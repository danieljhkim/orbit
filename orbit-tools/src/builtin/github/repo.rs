use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct GithubRepoViewTool;

pub(super) fn build_exec_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    let mut args = vec!["repo".to_string(), "view".to_string()];

    if let Some(repo) = input.get("repo").and_then(Value::as_str) {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }

    args.push("--json".to_string());
    args.push("name,defaultBranchRef".to_string());

    Ok(ExecRequest {
        program: "gh".to_string(),
        args,
        current_dir: None,
        timeout_ms: Some(15_000),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    })
}

impl Tool for GithubRepoViewTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "github.repo.view".to_string(),
            description: "Retrieve repository metadata including name and default branch"
                .to_string(),
            parameters: vec![ToolParam {
                name: "repo".to_string(),
                description: "Repository in owner/name format (uses current directory if omitted)"
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            }],
            builtin: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(&input)?;
        let result = run_process(&req, &NoSandbox)?;

        if !result.success {
            return Err(OrbitError::Execution(format!(
                "gh repo view failed: {}",
                result.stderr.trim()
            )));
        }

        let parsed: Value = serde_json::from_str(&result.stdout).map_err(|e| {
            OrbitError::Execution(format!("failed to parse gh repo view output: {e}"))
        })?;

        Ok(json!({
            "name": parsed["name"],
            "default_branch": parsed["defaultBranchRef"]["name"],
        }))
    }
}
