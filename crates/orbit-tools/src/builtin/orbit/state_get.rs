use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

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
                    description: "Optional active run ID when state_dir is not provided"
                        .to_string(),
                    param_type: "string".to_string(),
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
        let state_dir = super::resolve_state_dir(ctx, &input)?;
        let pipeline = orbit_store::state_io::read_pipeline(&state_dir)?;
        match super::optional_string(&input, "key")? {
            Some(key) => Ok(pipeline
                .as_object()
                .and_then(|map| map.get(&key))
                .cloned()
                .unwrap_or(Value::Null)),
            None => Ok(pipeline),
        }
    }
}
