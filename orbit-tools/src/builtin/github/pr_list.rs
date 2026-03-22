use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{TIMEOUT_DEFAULT_MS, Tool, ToolContext, check_exec_result};

pub struct GithubPrListTool;

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

impl Tool for GithubPrListTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "github.pr.list".to_string(),
            description: "List pull requests, optionally filtered by label and state".to_string(),
            parameters: vec![
                ToolParam {
                    name: "label".to_string(),
                    description: "Filter by label (e.g. \"orbit\")".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "state".to_string(),
                    description: "PR state filter: open (default), closed, or merged".to_string(),
                    param_type: "string".to_string(),
                    required: false,
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
        check_exec_result(&result, "gh pr list")?;

        let prs: Value = serde_json::from_str(&result.stdout).map_err(|e| {
            OrbitError::Execution(format!("failed to parse gh pr list output: {e}"))
        })?;

        Ok(json!({ "pull_requests": prs }))
    }
}
