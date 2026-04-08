use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitTaskRejectTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let id = super::required_string(input, &["id"], "id")?;
    let note = super::required_string(input, &["note"], "note")?;

    let mut args = vec![
        "task".to_string(),
        "reject".to_string(),
        id,
        "--note".to_string(),
        note,
    ];

    if let Some(comment) = super::optional_string(input, "comment")? {
        args.push("--comment".to_string());
        args.push(comment);
    }
    super::append_identity_flags(&mut args, &identity);

    args.push("--json".to_string());
    Ok(super::orbit_exec_request_with_identity(
        ctx, args, &identity,
    ))
}

impl Tool for OrbitTaskRejectTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.extend([
            ToolParam {
                name: "note".to_string(),
                description: "Rejection note".to_string(),
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
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.task.reject".to_string(),
            description: "Reject an Orbit task and return the updated task JSON".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        super::run_orbit_json_command(req, "orbit task reject")
    }
}
