use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitReviewThreadReplyTool;

impl Tool for OrbitReviewThreadReplyTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.push(ToolParam {
            name: "thread_id".to_string(),
            description: "Review thread ID to reply to".to_string(),
            param_type: "string".to_string(),
            required: true,
        });
        parameters.push(ToolParam {
            name: "body".to_string(),
            description: "Reply body".to_string(),
            param_type: "string".to_string(),
            required: true,
        });
        parameters.extend(super::scored_identity_params());

        ToolSchema {
            name: "orbit.task.review_thread.reply".to_string(),
            description: "Reply to an existing review thread on an Orbit task".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::ReviewThreadReply)
    }
}
