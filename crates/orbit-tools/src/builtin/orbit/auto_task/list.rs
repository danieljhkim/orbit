use orbit_common::OrbitError;
use orbit_types::tool::ToolSchema;
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitAutoTaskListTool;

impl Tool for OrbitAutoTaskListTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.auto_task.list".to_string(),
            description: "List every auto-task definition in this workspace.".to_string(),
            parameters: vec![],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::AutoTaskList)
    }
}
