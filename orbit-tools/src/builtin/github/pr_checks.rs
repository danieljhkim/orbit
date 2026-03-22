use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, Tool, ToolContext, check_exec_result};

pub struct GithubPrChecksTool;

pub(super) fn build_exec_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    let pr = super::require_pr(input)?;

    let mut args = vec![
        "pr".to_string(),
        "checks".to_string(),
        pr,
        "--json".to_string(),
        "state,name".to_string(),
    ];

    if let Some(repo) = input.get("repo").and_then(Value::as_str) {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }

    Ok(ExecRequest {
        program: "gh".to_string(),
        args,
        current_dir: None,
        timeout_ms: Some(TIMEOUT_DEFAULT_MS),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    })
}

impl Tool for GithubPrChecksTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "github.pr.checks".to_string(),
            description: "Get CI check status for a pull request".to_string(),
            parameters: vec![
                ToolParam {
                    name: "pr".to_string(),
                    description: "PR number, URL, or branch name".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "repo".to_string(),
                    description: "Repository in owner/name format".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(&input)?;
        let result = run_process(&req, &NoSandbox)?;
        check_exec_result(&result, "gh pr checks")?;

        let checks: Value = serde_json::from_str(&result.stdout).map_err(|e| {
            OrbitError::Execution(format!("failed to parse gh pr checks output: {e}"))
        })?;

        Ok(json!({ "checks": checks }))
    }
}
