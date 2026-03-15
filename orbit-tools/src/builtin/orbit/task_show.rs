use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitTaskShowTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let id = super::required_string(input, &["id"], "id")?;
    Ok(super::orbit_exec_request(
        ctx,
        vec![
            "task".to_string(),
            "show".to_string(),
            id,
            "--json".to_string(),
        ],
    ))
}

impl Tool for OrbitTaskShowTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.task.show".to_string(),
            description: "Fetch a single Orbit task as JSON".to_string(),
            parameters: super::orbit_id_params("task"),
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        super::run_orbit_json_command(ctx, req.args, "orbit task show")
    }
}
