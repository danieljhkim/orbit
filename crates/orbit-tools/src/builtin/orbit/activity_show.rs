use orbit_types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitActivityShowTool;

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
        super::execute_host_action(ctx, input, OrbitBuiltinAction::ActivityShow)
    }
}
