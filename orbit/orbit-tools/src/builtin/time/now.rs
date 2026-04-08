use orbit_types::{OrbitError, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct TimeNowTool;

impl Tool for TimeNowTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "time.now".to_string(),
            description: "Return current UTC timestamp".to_string(),
            parameters: vec![],
            builtin: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, _input: Value) -> Result<Value, OrbitError> {
        Ok(json!({"now": chrono::Utc::now().to_rfc3339()}))
    }
}
