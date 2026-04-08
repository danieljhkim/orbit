use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitJobRunListTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let mut args = vec![
        "job-run".to_string(),
        "list".to_string(),
        "--json".to_string(),
    ];

    if let Some(job) = super::optional_string(input, "job")? {
        args.push("--job".to_string());
        args.push(job);
    }
    if let Some(status) = super::optional_string(input, "status")? {
        args.push("--status".to_string());
        args.push(status);
    }
    if let Some(since) = super::optional_string(input, "since")? {
        args.push("--since".to_string());
        args.push(since);
    }
    if let Some(limit) = super::optional_string(input, "limit")? {
        args.push("--limit".to_string());
        args.push(limit);
    }

    Ok(super::orbit_exec_request_with_identity(
        ctx, args, &identity,
    ))
}

impl Tool for OrbitJobRunListTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "job".to_string(),
                description: "Optional job ID filter".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "status".to_string(),
                description: "Optional job run status filter".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "since".to_string(),
                description: "Optional lower timestamp bound".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "limit".to_string(),
                description: "Optional result limit".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::identity_params());
        ToolSchema {
            name: "orbit.job_run.list".to_string(),
            description: "List Orbit job runs as JSON".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        let output = super::run_orbit_json_command(req, "orbit job-run list")?;
        if !output.is_array() {
            return Err(OrbitError::Execution(
                "failed to parse orbit job-run list output: expected JSON array".to_string(),
            ));
        }
        Ok(output)
    }
}
