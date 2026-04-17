use orbit_types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitTaskDeleteTool;

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
        super::execute_host_action(ctx, input, OrbitBuiltinAction::TaskDelete)
    }
}
