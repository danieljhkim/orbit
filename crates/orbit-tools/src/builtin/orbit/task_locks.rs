use orbit_types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitTaskLocksTool;

impl Tool for OrbitTaskLocksTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.task.locks".to_string(),
            description: "List files currently locked by active Orbit tasks as JSON.".to_string(),
            parameters: vec![],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::TaskLocks)
    }
}
