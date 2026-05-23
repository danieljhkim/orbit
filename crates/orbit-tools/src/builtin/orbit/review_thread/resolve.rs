use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitReviewThreadResolveTool;
pub struct OrbitReviewThreadResolveAliasTool;

impl Tool for OrbitReviewThreadResolveTool {
    fn schema(&self) -> ToolSchema {
        resolve_schema("orbit.task.review_thread.resolve")
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::ReviewThreadResolve)
    }
}

impl Tool for OrbitReviewThreadResolveAliasTool {
    fn schema(&self) -> ToolSchema {
        resolve_schema("orbit.review-thread.resolve")
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::ReviewThreadResolve)
    }
}

fn resolve_schema(name: &str) -> ToolSchema {
    let mut parameters = super::super::orbit_id_params("task");
    parameters.push(ToolParam {
        name: "task_id".to_string(),
        description: "Task ID alias for id".to_string(),
        param_type: "string".to_string(),
        required: false,
    });
    parameters.push(ToolParam {
        name: "thread_id".to_string(),
        description: "Review thread ID to resolve".to_string(),
        param_type: "string".to_string(),
        required: true,
    });
    parameters.extend(super::super::identity_params());

    ToolSchema {
        name: name.to_string(),
        description: "Resolve a review thread on an Orbit task".to_string(),
        parameters,
        builtin: true,
    }
}
