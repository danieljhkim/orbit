use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitTaskDeleteTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let id = super::required_string(input, &["id"], "id")?;

    let mut args = vec![
        "task".to_string(),
        "delete".to_string(),
        id,
        "--force".to_string(),
    ];

    args.push("--json".to_string());
    Ok(super::orbit_exec_request_with_identity(
        ctx, args, &identity,
    ))
}

impl Tool for OrbitTaskDeleteTool {
    fn schema(&self) -> ToolSchema {
        let parameters = super::orbit_id_params("task");

        ToolSchema {
            name: "orbit.task.delete".to_string(),
            description: "Permanently delete an Orbit task and return confirmation JSON"
                .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        super::run_orbit_json_command(req, "orbit task delete")
    }
}
