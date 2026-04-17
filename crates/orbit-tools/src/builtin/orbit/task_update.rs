use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitTaskUpdateTool;

impl Tool for OrbitTaskUpdateTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.extend([
            ToolParam {
                name: "title".to_string(),
                description: "New task title".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "description".to_string(),
                description: "New task description (empty string clears)".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "acceptance_criteria".to_string(),
                description: "New acceptance criteria as an array of strings or a single string"
                    .to_string(),
                param_type: "array".to_string(),
                required: false,
            },
            ToolParam {
                name: "plan".to_string(),
                description: "Replacement task plan text (empty string clears)".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "status".to_string(),
                description: "New task status".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "execution_summary".to_string(),
                description: "Replacement execution summary text".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "comment".to_string(),
                description: "Task comment to append".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "pr_status".to_string(),
                description: "PR review status (e.g. approve, request-changes)".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "pr_number".to_string(),
                description: "Pull request number (empty string clears)".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "batch_id".to_string(),
                description: "Batch ID to associate with the task (empty string clears)"
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "context_files".to_string(),
                description: "Context file paths as a comma-separated string or array of strings"
                    .to_string(),
                param_type: "array".to_string(),
                required: false,
            },
            ToolParam {
                name: "artifacts".to_string(),
                description:
                    "Task artifacts to write under `artifacts/`. Accepts either an object \
                    map of `path -> content` or an array of `{ path, content }` objects."
                        .to_string(),
                param_type: "object".to_string(),
                required: false,
            },
        ]);
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.task.update".to_string(),
            description: "Update an Orbit task and return the fresh task JSON".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::TaskUpdate)
    }
}
