use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitStateGetTool;

impl Tool for OrbitStateGetTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.state.get".to_string(),
            description: "Read persisted pipeline state for an active run".to_string(),
            parameters: vec![
                ToolParam {
                    name: "key".to_string(),
                    description: "Optional pipeline key to read".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "run_id".to_string(),
                    description:
                        "Optional run ID; managed activity calls must match the active run"
                            .to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "state_dir".to_string(),
                    description:
                        "Optional active run bundle directory; must resolve to the current run"
                            .to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::StateGet)
    }
}
