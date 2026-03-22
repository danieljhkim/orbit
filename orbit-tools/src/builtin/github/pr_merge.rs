use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{TIMEOUT_SLOW_MS, Tool, ToolContext, check_exec_result};

pub struct GithubPrMergeTool;

pub(super) fn build_exec_request(input: &Value) -> Result<ExecRequest, OrbitError> {
    let pr = super::require_pr(input)?;

    let strategy = input
        .get("strategy")
        .and_then(Value::as_str)
        .unwrap_or("squash");

    let strategy_flag = match strategy {
        "squash" => "--squash",
        "merge" => "--merge",
        "rebase" => "--rebase",
        other => {
            return Err(OrbitError::InvalidInput(format!(
                "invalid `strategy`: \"{other}\"; must be squash, merge, or rebase"
            )));
        }
    };

    let delete_branch = input
        .get("delete_branch")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let mut args = vec![
        "pr".to_string(),
        "merge".to_string(),
        pr,
        strategy_flag.to_string(),
    ];

    if delete_branch {
        args.push("--delete-branch".to_string());
    }

    if let Some(repo) = input.get("repo").and_then(Value::as_str) {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }

    Ok(ExecRequest {
        program: "gh".to_string(),
        args,
        current_dir: None,
        timeout_ms: Some(TIMEOUT_SLOW_MS),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    })
}

impl Tool for GithubPrMergeTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "github.pr.merge".to_string(),
            description: "Merge a pull request".to_string(),
            parameters: vec![
                ToolParam {
                    name: "pr".to_string(),
                    description: "PR number, URL, or branch name".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "strategy".to_string(),
                    description: "Merge strategy: squash (default), merge, or rebase".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "delete_branch".to_string(),
                    description: "Delete the head branch after merge (default: true)".to_string(),
                    param_type: "bool".to_string(),
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
        check_exec_result(&result, "gh pr merge")?;
        Ok(json!({
            "stdout": result.stdout,
            "stderr": result.stderr,
        }))
    }
}
