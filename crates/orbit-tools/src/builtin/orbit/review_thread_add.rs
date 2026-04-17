use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitReviewThreadAddTool;

impl Tool for OrbitReviewThreadAddTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.push(ToolParam {
            name: "body".to_string(),
            description: "Review comment body".to_string(),
            param_type: "string".to_string(),
            required: true,
        });
        parameters.push(ToolParam {
            name: "path".to_string(),
            description: "File path for inline review comment".to_string(),
            param_type: "string".to_string(),
            required: false,
        });
        parameters.push(ToolParam {
            name: "line".to_string(),
            description: "Line number for inline review comment".to_string(),
            param_type: "string".to_string(),
            required: false,
        });
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.task.review_thread.add".to_string(),
            description: "Create a new review thread on an Orbit task".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::ReviewThreadAdd)
    }
}
