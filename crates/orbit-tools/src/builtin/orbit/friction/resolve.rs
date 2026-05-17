use orbit_common::types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitFrictionResolveTool;

impl Tool for OrbitFrictionResolveTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.friction.resolve".to_string(),
            description: "Mark an Orbit friction record as resolved".to_string(),
            parameters: super::super::orbit_id_params("friction"),
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::FrictionResolve)
    }
}
