use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitAutoTaskToggleTool;

impl Tool for OrbitAutoTaskToggleTool {
    fn schema(&self) -> ToolSchema {
        let parameters = vec![
            ToolParam {
                name: "name".to_string(),
                description: "Definition name. Required.".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "enabled".to_string(),
                description: "Whether to enable (`true`) or disable (`false`) the definition. Disabling is the kill-switch, not a delete. Required.".to_string(),
                param_type: "boolean".to_string(),
                required: true,
            },
        ];
        ToolSchema {
            name: "orbit.auto_task.toggle".to_string(),
            description: "Enable or disable an auto-task definition without deleting it."
                .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::AutoTaskToggle)
    }
}
