use std::fs;
use std::path::Path;

use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct FsReadTool;

impl Tool for FsReadTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.read".to_string(),
            description: "Read a UTF-8 text file from disk".to_string(),
            parameters: vec![ToolParam {
                name: "path".to_string(),
                description: "Path to the file to read".to_string(),
                param_type: "string".to_string(),
                required: true,
            }],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let path_str = input
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| OrbitError::InvalidInput("missing `path`".to_string()))?;

        let canonical = super::check_workspace_boundary(ctx, Path::new(path_str))?;
        let content = fs::read_to_string(&canonical).map_err(|e| OrbitError::Io(e.to_string()))?;

        Ok(json!({
            "path": canonical.display().to_string(),
            "content": content,
        }))
    }
}
