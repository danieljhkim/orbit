use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitReviewThreadAddTool;
pub struct OrbitReviewThreadAddAliasTool;

impl Tool for OrbitReviewThreadAddTool {
    fn schema(&self) -> ToolSchema {
        add_schema("orbit.task.review_thread.add")
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::ReviewThreadAdd)
    }
}

impl Tool for OrbitReviewThreadAddAliasTool {
    fn schema(&self) -> ToolSchema {
        add_schema("orbit.review-thread.add")
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::ReviewThreadAdd)
    }
}

fn add_schema(name: &str) -> ToolSchema {
    let mut parameters = super::super::orbit_id_params("task");
    parameters.push(ToolParam {
        name: "task_id".to_string(),
        description: "Task ID alias for id".to_string(),
        param_type: "string".to_string(),
        required: false,
    });
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
    parameters.extend(super::super::scored_identity_params());

    ToolSchema {
        name: name.to_string(),
        description: "Create a new review thread on an Orbit task".to_string(),
        parameters,
        builtin: true,
    }
}
