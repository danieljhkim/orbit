use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitLearningCommentDeleteTool;

impl Tool for OrbitLearningCommentDeleteTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![ToolParam {
            name: "id".to_string(),
            description: "ID of the learning comment to soft-delete.".to_string(),
            param_type: "string".to_string(),
            required: true,
        }];
        parameters.extend(super::super::model_identity_params());
        ToolSchema {
            name: "orbit.learning.comment.delete".to_string(),
            description:
                "Soft-delete a learning comment by appending a tombstone. Repeated deletes are idempotent."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::LearningCommentDelete)
    }
}
