use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitTaskListTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let mut args = vec!["task".to_string(), "list".to_string(), "--json".to_string()];

    if let Some(status) = super::optional_string(input, "status")? {
        args.push("--status".to_string());
        args.push(status);
    }

    Ok(super::orbit_exec_request(ctx, args))
}

impl Tool for OrbitTaskListTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.task.list".to_string(),
            description: "List Orbit tasks, optionally filtered by status".to_string(),
            parameters: vec![ToolParam {
                name: "status".to_string(),
                description: "Optional task status filter".to_string(),
                param_type: "string".to_string(),
                required: false,
            }],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        let output = super::run_orbit_json_command(ctx, req.args, "orbit task list")?;
        if !output.is_array() {
            return Err(OrbitError::Execution(
                "failed to parse orbit task list output: expected JSON array".to_string(),
            ));
        }
        Ok(output)
    }
}
