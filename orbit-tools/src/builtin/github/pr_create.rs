use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{TIMEOUT_SLOW_MS, Tool, ToolContext, check_exec_result, require_str};

pub struct GithubPrCreateTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let title = require_str(input, "title")?;
    let base = require_str(input, "base")?;
    let head = require_str(input, "head")?;

    let body = input.get("body").and_then(Value::as_str);
    let body_file = input.get("body_file").and_then(Value::as_str);

    if body.is_none() && body_file.is_none() {
        return Err(OrbitError::InvalidInput(
            "one of `body` or `body_file` is required".to_string(),
        ));
    }

    let mut args = vec![
        "pr".to_string(),
        "create".to_string(),
        "--title".to_string(),
        title.to_string(),
        "--base".to_string(),
        base.to_string(),
        "--head".to_string(),
        head.to_string(),
    ];

    if let Some(b) = body {
        args.push("--body".to_string());
        args.push(b.to_string());
    } else if let Some(f) = body_file {
        args.push("--body-file".to_string());
        args.push(f.to_string());
    }

    let label = input
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("orbit");
    args.push("--label".to_string());
    args.push(label.to_string());

    if let Some(repo) = input.get("repo").and_then(Value::as_str) {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }

    Ok(ExecRequest {
        program: "gh".to_string(),
        args,
        current_dir: ctx.cwd.clone(),
        timeout_ms: Some(TIMEOUT_SLOW_MS),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    })
}

impl Tool for GithubPrCreateTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "github.pr.create".to_string(),
            description: "Create a pull request".to_string(),
            parameters: vec![
                ToolParam {
                    name: "title".to_string(),
                    description: "Pull request title".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "base".to_string(),
                    description: "Base branch to merge into".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "head".to_string(),
                    description: "Head branch containing the changes".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "body".to_string(),
                    description: "Pull request body text (required if body_file is absent)"
                        .to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "body_file".to_string(),
                    description:
                        "Path to a file containing the PR body (required if body is absent)"
                            .to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "label".to_string(),
                    description: "Label to apply (defaults to \"orbit\")".to_string(),
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

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        let result = run_process(&req, &NoSandbox)?;
        check_exec_result(&result, "gh pr create")?;
        Ok(json!({
            "url": result.stdout.trim(),
            "stdout": result.stdout,
            "stderr": result.stderr,
        }))
    }
}
