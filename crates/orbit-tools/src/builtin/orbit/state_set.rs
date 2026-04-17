use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

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
        let state_dir = super::resolve_state_dir(ctx, &input)?;
        let step_index = super::resolve_step_index(&input)?;
        let payload = resolve_payload(&input)?;
        orbit_store::state_io::write_step_output(&state_dir, step_index, &payload)?;
        Ok(json!({
            "state_dir": state_dir.display().to_string(),
            "step_index": step_index,
            "written": payload,
        }))
    }
}

fn resolve_payload(input: &Value) -> Result<Value, OrbitError> {
    let data = input.get("data");
    let key = super::optional_string(input, "key")?;
    let value = input.get("value");
    match (data, key, value) {
        (Some(_), Some(_), _) => Err(OrbitError::InvalidInput(
            "provide either `data` or `key`/`value`, not both".to_string(),
        )),
        (Some(data), None, None) => {
            if !data.is_object() {
                return Err(OrbitError::InvalidInput(
                    "`data` must be a JSON object".to_string(),
                ));
            }
            Ok(data.clone())
        }
        (None, Some(key), Some(value)) => Ok(json!({ key: value.clone() })),
        (None, Some(_), None) => Err(OrbitError::InvalidInput(
            "`value` is required when `key` is provided".to_string(),
        )),
        (None, None, Some(_)) => Err(OrbitError::InvalidInput(
            "`key` is required when `value` is provided".to_string(),
        )),
        (None, None, None) => Err(OrbitError::InvalidInput(
            "provide either `data` or `key`/`value`".to_string(),
        )),
        (Some(_), None, Some(_)) => Err(OrbitError::InvalidInput(
            "provide either `data` or `key`/`value`, not both".to_string(),
        )),
    }
}
