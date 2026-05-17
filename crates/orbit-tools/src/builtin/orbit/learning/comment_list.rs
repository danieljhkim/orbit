use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitLearningCommentListTool;

impl Tool for OrbitLearningCommentListTool {
    fn schema(&self) -> ToolSchema {
        let parameters = vec![
            ToolParam {
                name: "learning_id".to_string(),
                description: "ID of the parent learning.".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "include_deleted".to_string(),
                description:
                    "When true, include comments that have a later delete tombstone. Defaults to false."
                        .to_string(),
                param_type: "boolean".to_string(),
                required: false,
            },
        ];
        ToolSchema {
            name: "orbit.learning.comment.list".to_string(),
            description:
                "List comments for one learning, oldest first. Deleted comments are hidden unless requested."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::LearningCommentList)
    }
}
