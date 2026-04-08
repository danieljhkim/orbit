use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitReviewThreadResolveTool;

pub(super) fn build_exec_requests(
    ctx: &ToolContext,
    input: &Value,
) -> Result<(ExecRequest, ExecRequest), OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let id = super::required_string(input, &["id"], "id")?;
    let thread_id = super::required_string(input, &["thread_id"], "thread_id")?;

    let mut args = vec![
        "task".to_string(),
        "review-thread".to_string(),
        "resolve".to_string(),
        id.clone(),
        thread_id,
    ];

    super::append_identity_flags(&mut args, &identity);
    args.push("--json".to_string());

    let resolve = super::orbit_exec_request_with_identity(ctx, args, &identity);
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
    Ok((resolve, show))
}

impl Tool for OrbitReviewThreadResolveTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.push(ToolParam {
            name: "thread_id".to_string(),
            description: "Review thread ID to resolve".to_string(),
            param_type: "string".to_string(),
            required: true,
        });
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.task.review_thread.resolve".to_string(),
            description: "Resolve a review thread on an Orbit task".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let (resolve_req, show_req) = build_exec_requests(ctx, &input)?;

        let resolve_result = orbit_exec::run_process(&resolve_req, &orbit_exec::NoSandbox)?;
        if !resolve_result.success {
            let stderr = resolve_result.stderr.trim();
            let detail = if stderr.is_empty() {
                "command returned non-zero exit status"
            } else {
                stderr
            };
            return Err(OrbitError::Execution(format!(
                "orbit task review-thread resolve failed: {detail}"
            )));
        }

        super::run_orbit_json_command(show_req, "orbit task show")
    }
}
