use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitTaskAddTool;

impl Tool for OrbitTaskAddTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "title".to_string(),
                description: "Task title".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "description".to_string(),
                description: "Task description markdown".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "acceptance_criteria".to_string(),
                description: "Optional acceptance criteria as a string or array of strings"
                    .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "dependencies".to_string(),
                description: "Optional dependency task IDs as a string or array of strings"
                    .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "tags".to_string(),
                description: "Optional tags as a string or array of strings".to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "plan".to_string(),
                description:
                    "Optional task plan markdown. Leave blank for the executing agent to author."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "workspace".to_string(),
                description: "Workspace path for the task".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "comment".to_string(),
                description: "Optional initial task comment".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "external_refs".to_string(),
                description:
                    "Optional external tracker refs as an array of {system, id, url?} objects"
                        .to_string(),
                param_type: "array".to_string(),
                required: false,
            },
            ToolParam {
                name: "context_files".to_string(),
                description:
                    "Optional task context selectors as a comma-separated string or array of strings. Prefer canonical selectors: `file:path`, `dir:path`, or `symbol:path#name:kind`. Legacy raw paths are accepted and upgraded automatically."
                        .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "context".to_string(),
                description:
                    "Legacy alias for `context_files`. Accepts the same selector-first input forms."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "priority".to_string(),
                description: "Optional priority level".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "complexity".to_string(),
                description: "Optional task complexity level".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "type".to_string(),
                description: "Optional task type".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "status".to_string(),
                description: "Optional initial task status".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "source_task_id".to_string(),
                description: "For bug tasks: originating task ID that introduced the defect"
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "parent_id".to_string(),
                description: "Optional parent task ID for a subtask relationship".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::super::identity_params());

        ToolSchema {
            name: "orbit.task.add".to_string(),
            description: "Create an Orbit task and return the created task JSON".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::TaskAdd)
    }
}
