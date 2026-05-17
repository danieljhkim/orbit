use orbit_common::types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitFrictionTagsTool;

impl Tool for OrbitFrictionTagsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.friction.tags".to_string(),
            description: "List configured friction taxonomy tags".to_string(),
            parameters: Vec::new(),
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::FrictionTags)
    }
}
