use orbit_types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct NetHttpTool;

impl Tool for NetHttpTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "net.http".to_string(),
            description: "HTTP tool placeholder for v2.1".to_string(),
            parameters: vec![],
            builtin: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, _input: Value) -> Result<Value, OrbitError> {
        Err(OrbitError::Execution(
            "net.http is not implemented in v2.1".to_string(),
        ))
    }
}
