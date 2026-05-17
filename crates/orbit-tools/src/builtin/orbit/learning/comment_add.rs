use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitLearningCommentAddTool;

impl Tool for OrbitLearningCommentAddTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "learning_id".to_string(),
                description: "ID of the parent learning.".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "body".to_string(),
                description: "Comment body (trimmed, non-empty, ≤ 500 chars).".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
        ];
        parameters.extend(super::super::model_identity_params());
        ToolSchema {
            name: "orbit.learning.comment.add".to_string(),
            description:
                "Append a footnote-style comment to an active learning. Returns `{ id, learning_id, created_at }`."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::LearningCommentAdd)
    }
}
