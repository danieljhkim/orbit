use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitTaskApproveTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let id = super::required_string(input, &["id"], "id")?;
    let note = super::required_string(input, &["note"], "note")?;

    let mut args = vec![
        "task".to_string(),
        "approve".to_string(),
        id,
        "--note".to_string(),
        note,
    ];

    if let Some(comment) = super::optional_string(input, "comment")? {
        args.push("--comment".to_string());
        args.push(comment);
    }

    args.push("--json".to_string());
    Ok(super::orbit_exec_request(ctx, args))
}

impl Tool for OrbitTaskApproveTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.extend([
            ToolParam {
                name: "note".to_string(),
                description: "Approval note".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "comment".to_string(),
                description: "Optional task comment to append".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ]);

        ToolSchema {
            name: "orbit.task.approve".to_string(),
            description: "Approve an Orbit task and return the updated task JSON".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        super::run_orbit_json_command(ctx, req.args, "orbit task approve")
    }
}
