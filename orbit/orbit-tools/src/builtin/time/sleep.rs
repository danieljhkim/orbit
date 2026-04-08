use std::thread;
use std::time::Duration;

use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct TimeSleepTool;

impl Tool for TimeSleepTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "time.sleep".to_string(),
            description: "Sleep for a number of milliseconds".to_string(),
            parameters: vec![ToolParam {
                name: "ms".to_string(),
                description: "Number of milliseconds to sleep".to_string(),
                param_type: "u64".to_string(),
                required: true,
            }],
            builtin: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let ms = input
            .get("ms")
            .and_then(Value::as_u64)
            .ok_or_else(|| OrbitError::InvalidInput("missing `ms`".to_string()))?;

        thread::sleep(Duration::from_millis(ms));
        Ok(json!({"slept_ms": ms}))
    }
}
