use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitStateSetTool;

impl Tool for OrbitStateSetTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.state.set".to_string(),
            description: "Write persisted step output for an active run".to_string(),
            parameters: vec![
                ToolParam {
                    name: "key".to_string(),
                    description: "Single key to write when not providing `data`".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "value".to_string(),
                    description: "JSON value to pair with `key`".to_string(),
                    param_type: "object".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "data".to_string(),
                    description: "JSON object to merge into this step's persisted output"
                        .to_string(),
                    param_type: "object".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "run_id".to_string(),
                    description: "Optional active run ID when state_dir is not provided"
                        .to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "step_index".to_string(),
                    description: "Optional step index when ORBIT_STEP_INDEX is not set".to_string(),
                    param_type: "integer".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "state_dir".to_string(),
                    description: "Optional active run bundle directory containing state.json"
                        .to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::StateSet)
    }
}
