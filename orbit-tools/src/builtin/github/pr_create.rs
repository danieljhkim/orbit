use std::path::Path;

use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::builtin::fs::check_workspace_boundary;
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
        args.push(super::append_signature(b, ctx, "Implemented"));
    } else if let Some(f) = body_file {
        let validated = check_workspace_boundary(ctx, Path::new(f))?;
        args.push("--body-file".to_string());
        args.push(validated.to_string_lossy().to_string());
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

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use super::*;
    use crate::ToolContext;

    fn base_input(body_file: &str) -> Value {
        json!({
            "title": "test pr",
            "base": "main",
            "head": "feature",
            "body_file": body_file,
        })
    }

    #[test]
    fn body_file_inside_workspace_is_accepted() {
        let workspace = tempdir().expect("workspace dir");
        let file = workspace.path().join("pr_body.md");
        fs::write(&file, "PR description").expect("write file");

        let ctx = ToolContext {
            workspace_root: Some(workspace.path().canonicalize().expect("canonicalize")),
            ..Default::default()
        };

        let req = build_exec_request(&ctx, &base_input(&file.to_string_lossy()))
            .expect("should accept body_file inside workspace");
        assert!(req.args.contains(&"--body-file".to_string()));
    }

    #[test]
    fn body_file_outside_workspace_is_denied() {
        let workspace = tempdir().expect("workspace dir");
        let outside = tempdir().expect("outside dir");
        let outside_file = outside.path().join("secret.txt");
        fs::write(&outside_file, "secret data").expect("write outside file");

        let ctx = ToolContext {
            workspace_root: Some(workspace.path().canonicalize().expect("canonicalize")),
            ..Default::default()
        };

        let err = build_exec_request(&ctx, &base_input(&outside_file.to_string_lossy()))
            .expect_err("body_file outside workspace must be denied");
        assert!(
            err.to_string().contains("outside workspace"),
            "expected policy denied message, got: {err}"
        );
    }

    #[test]
    fn body_file_denied_when_workspace_root_is_none() {
        let dir = tempdir().expect("temp dir");
        let file = dir.path().join("body.md");
        fs::write(&file, "content").expect("write file");

        let err = build_exec_request(
            &ToolContext::default(),
            &base_input(&file.to_string_lossy()),
        )
        .expect_err("body_file with no workspace_root must be denied");
        assert!(
            err.to_string().contains("workspace_root is not set"),
            "expected fail-closed denial, got: {err}"
        );
    }
}
