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
                name: "relations".to_string(),
                description:
                    "Optional typed task relations as an array of {type, target} objects"
                        .to_string(),
                param_type: "array".to_string(),
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
            // ADR-0149: `workspace` is the binding key for ~/.orbit/tasks/workspaces/<id>/
            // home-store projection; defaulting to cwd would silently misroute tasks under
            // worktrees, subdirectories, or non-default `workspace_id` in .orbit/config.yaml.
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
                    "Optional task context selectors as a comma-separated string or array of strings. Add entries ONLY for existing files, directories, or symbols expected to be modified or deleted by the task. Do not add background-reading entries or files referenced only for context. Prefer canonical selectors: `file:path`, `dir:path`, or `symbol:path#name:kind`. Legacy raw paths are accepted and upgraded automatically."
                        .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "context".to_string(),
                description:
                    "Legacy alias for `context_files`. Add entries ONLY for existing files, directories, or symbols expected to be modified or deleted by the task. Do not add background-reading entries or files that are only relevant background context. Prefer canonical selectors: `file:path`, `dir:path`, or `symbol:path#name:kind`."
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
                description: "Optional task complexity level (low, medium, or hard)".to_string(),
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
            ToolParam {
                name: "crew".to_string(),
                description: "Optional named crew to use when running this task".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::super::model_identity_params());

        ToolSchema {
            name: "orbit.task.add".to_string(),
            description: "Create an Orbit task and return the created task JSON".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::reject_agent_field(&input, "orbit.task.add")?;
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::TaskAdd)
    }
}

#[cfg(test)]
mod tests;
