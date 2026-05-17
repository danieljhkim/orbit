use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitLearningUpvoteTool;

impl Tool for OrbitLearningUpvoteTool {
    fn schema(&self) -> ToolSchema {
        let parameters = vec![
            ToolParam {
                name: "id".to_string(),
                description: "ID of the learning being re-validated.".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "model".to_string(),
                description:
                    "Voter model or canonical agent family (`codex`, `claude`, `gemini`, `grok`)."
                        .to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "task".to_string(),
                description:
                    "Task ID anchoring the vote. Required by the v1 free-floating vote policy."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "task_id".to_string(),
                description: "Alias for `task`.".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        ToolSchema {
            name: "orbit.learning.upvote".to_string(),
            description:
                "Record a task-anchored upvote for an active learning. Duplicate `(learning, model, task)` votes are idempotent."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::LearningUpvote)
    }
}
