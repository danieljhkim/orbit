use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitReviewThreadAddTool;

pub(super) fn build_exec_requests(
    ctx: &ToolContext,
    input: &Value,
) -> Result<(ExecRequest, ExecRequest), OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let id = super::required_string(input, &["id"], "id")?;
    let body = super::required_string(input, &["body"], "body")?;

    let mut args = vec![
        "task".to_string(),
        "review-thread".to_string(),
        "add".to_string(),
        id.clone(),
        "--body".to_string(),
        body,
    ];

    if let Some(path) = super::optional_string(input, "path")? {
        args.push("--path".to_string());
        args.push(path);
    }
    if let Some(line) = super::optional_string(input, "line")? {
        args.push("--line".to_string());
        args.push(line);
    }

    super::append_identity_flags(&mut args, &identity);
    args.push("--json".to_string());

    let add = super::orbit_exec_request_with_identity(ctx, args, &identity);
    let show = super::orbit_exec_request_with_identity(
        ctx,
        vec![
            "task".to_string(),
            "show".to_string(),
            id,
            "--json".to_string(),
        ],
        &identity,
    );
    Ok((add, show))
}

impl Tool for OrbitReviewThreadAddTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.push(ToolParam {
            name: "body".to_string(),
            description: "Review comment body".to_string(),
            param_type: "string".to_string(),
            required: true,
        });
        parameters.push(ToolParam {
            name: "path".to_string(),
            description: "File path for inline review comment".to_string(),
            param_type: "string".to_string(),
            required: false,
        });
        parameters.push(ToolParam {
            name: "line".to_string(),
            description: "Line number for inline review comment".to_string(),
            param_type: "string".to_string(),
            required: false,
        });
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.task.review_thread.add".to_string(),
            description: "Create a new review thread on an Orbit task".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let (add_req, show_req) = build_exec_requests(ctx, &input)?;

        let add_result = orbit_exec::run_process(&add_req, &orbit_exec::NoSandbox)?;
        if !add_result.success {
            let stderr = add_result.stderr.trim();
            let detail = if stderr.is_empty() {
                "command returned non-zero exit status"
            } else {
                stderr
            };
            return Err(OrbitError::Execution(format!(
                "orbit task review-thread add failed: {detail}"
            )));
        }

        super::run_orbit_json_command(show_req, "orbit task show")
    }
}
