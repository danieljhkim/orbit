use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitReviewThreadListTool;

impl Tool for OrbitReviewThreadListTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.push(ToolParam {
            name: "status".to_string(),
            description: "Filter by thread status: open or resolved".to_string(),
            param_type: "string".to_string(),
            required: false,
        });
        parameters.extend(super::identity_params());

        ToolSchema {
            name: "orbit.task.review_thread.list".to_string(),
            description: "List review threads on an Orbit task".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::ReviewThreadList)
    }
}
