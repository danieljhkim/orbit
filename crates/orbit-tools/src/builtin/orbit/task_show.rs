use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitTaskShowTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let id = super::required_string(input, &["id"], "id")?;
    let mut args = vec![
        "task".to_string(),
        "show".to_string(),
        id,
        "--json".to_string(),
    ];
    if let Some(fields) = super::optional_string_list_alias(input, &["fields", "field"])? {
        for field in fields {
            args.push("--fields".to_string());
            args.push(field);
        }
    }
    Ok(super::orbit_exec_request_with_identity(
        ctx, args, &identity,
    ))
}

impl Tool for OrbitTaskShowTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.extend(super::identity_params());
        parameters.push(ToolParam {
            name: "fields".to_string(),
            description:
                "Optional field projection as a string or array of strings. When set, returns only \
                the requested field(s) as JSON. Valid values: comments, plan, execution_summary, \
                description, acceptance_criteria, history, context_files, artifacts."
                    .to_string(),
            param_type: "array".to_string(),
            required: false,
        });
        parameters.push(ToolParam {
            name: "field".to_string(),
            description:
                "Compatibility alias for a single field projection. Example: `field: \"artifacts\"`."
                    .to_string(),
            param_type: "string".to_string(),
            required: false,
        });
        ToolSchema {
            name: "orbit.task.show".to_string(),
            description: "Fetch a single Orbit task as JSON. Use the optional `fields` projection \
                (or single-field alias `field`) to retrieve only specific task fields, such as \
                `field: \"artifacts\"`."
                .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        super::run_orbit_json_command(req, "orbit task show")
    }
}
