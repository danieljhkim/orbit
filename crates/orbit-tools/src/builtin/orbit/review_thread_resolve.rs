use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitReviewThreadResolveTool;

impl Tool for OrbitReviewThreadResolveTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.push(ToolParam {
            name: "thread_id".to_string(),
            description: "Review thread ID to resolve".to_string(),
            param_type: "string".to_string(),
            required: true,
        });
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.task.review_thread.resolve".to_string(),
            description: "Resolve a review thread on an Orbit task".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::ReviewThreadResolve)
    }
}
