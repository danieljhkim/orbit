use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitTaskLintTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let id = super::required_string(input, &["id"], "id")?;
    let mut args = vec![
        "task".to_string(),
        "lint".to_string(),
        id,
        "--json".to_string(),
    ];
    super::append_identity_flags(&mut args, &identity);
    Ok(super::orbit_exec_request_with_identity(
        ctx, args, &identity,
    ))
}

impl Tool for OrbitTaskLintTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.extend(super::identity_params());
        ToolSchema {
            name: "orbit.task.lint".to_string(),
            description: "Lint an Orbit task for stale paths, vague acceptance criteria, and stale repository names.".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        super::run_orbit_json_command(req, "orbit task lint")
    }
}
