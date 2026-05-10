use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
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
                name: "type".to_string(),
                description: "Optional task type filter".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "batch_id".to_string(),
                description: "Filter by batch ID".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "tag".to_string(),
                description: "Filter by tag. Repeat or pass an array for AND semantics."
                    .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "ready".to_string(),
                description: "When true, keep only tasks whose dependencies are satisfied"
                    .to_string(),
                param_type: "boolean".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::super::identity_params());
        ToolSchema {
            name: "orbit.task.list".to_string(),
            description:
                "List Orbit tasks, optionally filtered by status, parent, type, or dependency readiness"
                .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::TaskList)
    }
}
