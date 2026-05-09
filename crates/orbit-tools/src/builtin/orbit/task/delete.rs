use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitTaskDeleteTool;

impl Tool for OrbitTaskDeleteTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::super::orbit_id_params("task");
        parameters.push(ToolParam {
            name: "force".to_string(),
            description:
                "When true, allow permanent deletion outside proposed, friction, or rejected status"
                    .to_string(),
            param_type: "boolean".to_string(),
            required: false,
        });

        ToolSchema {
            name: "orbit.task.delete".to_string(),
            description: "Permanently delete an Orbit task and return confirmation JSON"
                .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::TaskDelete)
    }
}
