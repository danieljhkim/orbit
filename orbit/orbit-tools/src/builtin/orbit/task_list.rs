use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitTaskListTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let mut args = vec!["task".to_string(), "list".to_string(), "--json".to_string()];

    if let Some(status) = super::optional_string(input, "status")? {
        args.push("--status".to_string());
        args.push(status);
    }
    if let Some(parent_id) =
        super::optional_string_alias(input, &["parent_id", "parent", "parentId"])?
    {
        args.push("--parent".to_string());
        args.push(parent_id);
    }
    if let Some(batch_id) = super::optional_string(input, "batch_id")? {
        args.push("--batch-id".to_string());
        args.push(batch_id);
    }

    Ok(super::orbit_exec_request_with_identity(
        ctx, args, &identity,
    ))
}

impl Tool for OrbitTaskListTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "status".to_string(),
                description: "Optional task status filter".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "parent_id".to_string(),
                description: "Optional parent task ID to list subtasks for".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "batch_id".to_string(),
                description: "Filter by batch ID".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::identity_params());
        ToolSchema {
            name: "orbit.task.list".to_string(),
            description: "List Orbit tasks, optionally filtered by status or parent".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        let output = super::run_orbit_json_command(req, "orbit task list")?;
        if !output.is_array() {
            return Err(OrbitError::Execution(
                "failed to parse orbit task list output: expected JSON array".to_string(),
            ));
        }
        Ok(output)
    }
}
