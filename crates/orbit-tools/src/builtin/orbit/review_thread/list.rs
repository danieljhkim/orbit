use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitReviewThreadListTool;

impl Tool for OrbitReviewThreadListTool {
    fn schema(&self) -> ToolSchema {
        list_schema("orbit.task.review_thread.list")
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::ReviewThreadList)
    }
}

fn list_schema(name: &str) -> ToolSchema {
    let mut parameters = super::super::orbit_id_params("task");
    parameters.push(ToolParam {
        name: "task_id".to_string(),
        description: "Task ID alias for id".to_string(),
        param_type: "string".to_string(),
        required: false,
    });
    parameters.push(ToolParam {
        name: "status".to_string(),
        description: "Filter by thread status: open or resolved".to_string(),
        param_type: "string".to_string(),
        required: false,
    });
    parameters.extend(super::super::identity_params());

    ToolSchema {
        name: name.to_string(),
        description: "List review threads on an Orbit task".to_string(),
        parameters,
        builtin: true,
    }
}
