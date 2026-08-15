use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitAutoTaskMintTool;

impl Tool for OrbitAutoTaskMintTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.auto_task.mint".to_string(),
            description: "Mint one task now from an auto-task definition. Unconditional: the schedule, dedupe policy, and enabled flag are ignored, and the scheduler's own cursor is left untouched.".to_string(),
            parameters: vec![ToolParam {
                name: "name".to_string(),
                description: "Definition name. Required.".to_string(),
                param_type: "string".to_string(),
                required: true,
            }],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::AutoTaskMint)
    }
}
