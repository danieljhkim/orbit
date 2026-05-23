use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitReviewThreadReplyTool;
pub struct OrbitReviewThreadReplyAliasTool;

impl Tool for OrbitReviewThreadReplyTool {
    fn schema(&self) -> ToolSchema {
        reply_schema("orbit.task.review_thread.reply")
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::ReviewThreadReply)
    }
}

impl Tool for OrbitReviewThreadReplyAliasTool {
    fn schema(&self) -> ToolSchema {
        reply_schema("orbit.review-thread.reply")
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::ReviewThreadReply)
    }
}

fn reply_schema(name: &str) -> ToolSchema {
    let mut parameters = super::super::orbit_id_params("task");
    parameters.push(ToolParam {
        name: "task_id".to_string(),
        description: "Task ID alias for id".to_string(),
        param_type: "string".to_string(),
        required: false,
    });
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
    parameters.extend(super::super::scored_identity_params());

    ToolSchema {
        name: name.to_string(),
        description: "Reply to an existing review thread on an Orbit task".to_string(),
        parameters,
        builtin: true,
    }
}
