use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitActivityShowTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let id = super::required_string(input, &["id"], "id")?;
    Ok(super::orbit_exec_request_with_identity(
        ctx,
        vec![
            "activity".to_string(),
            "show".to_string(),
            id,
            "--json".to_string(),
        ],
        &identity,
    ))
}

impl Tool for OrbitActivityShowTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("activity");
        parameters.extend(super::identity_params());
        ToolSchema {
            name: "orbit.activity.show".to_string(),
            description: "Fetch a single Orbit activity as JSON".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        super::run_orbit_json_command(req, "orbit activity show")
    }
}
