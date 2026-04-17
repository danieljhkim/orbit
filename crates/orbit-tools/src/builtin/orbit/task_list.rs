use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitTaskListTool;

impl Tool for OrbitTaskListTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "status".to_string(),
                description: "Optional task status filter".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "parent_id".to_string(),
                description: "Optional parent task ID to list subtasks for".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "batch_id".to_string(),
                description: "Filter by batch ID".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::identity_params());
        ToolSchema {
            name: "orbit.task.list".to_string(),
            description: "List Orbit tasks, optionally filtered by status or parent".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::TaskList)
    }
}
