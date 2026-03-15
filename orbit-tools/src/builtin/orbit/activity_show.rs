use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitActivityShowTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let id = super::required_string(input, &["id", "activity_id"], "id")?;
    Ok(super::orbit_exec_request(
        ctx,
        vec![
            "activity".to_string(),
            "show".to_string(),
            id,
            "--json".to_string(),
        ],
    ))
}

impl Tool for OrbitActivityShowTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.activity.show".to_string(),
            description: "Fetch a single Orbit activity as JSON".to_string(),
            parameters: super::orbit_id_params("activity"),
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        super::run_orbit_json_command(ctx, req.args, "orbit activity show")
    }
}
