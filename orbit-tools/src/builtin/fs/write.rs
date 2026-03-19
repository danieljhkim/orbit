use std::fs;
use std::path::Path;

use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct FsWriteTool;

impl Tool for FsWriteTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.write".to_string(),
            description: "Write UTF-8 text content to disk".to_string(),
            parameters: vec![
                ToolParam {
                    name: "path".to_string(),
                    description: "Path to the file to write".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "content".to_string(),
                    description: "UTF-8 text content to write".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let path_str = input
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| OrbitError::InvalidInput("missing `path`".to_string()))?;
        let content = input
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| OrbitError::InvalidInput("missing `content`".to_string()))?;

        super::check_workspace_boundary(ctx, Path::new(path_str))?;

        fs::write(path_str, content).map_err(|e| OrbitError::Io(e.to_string()))?;

        Ok(json!({
            "path": path_str,
            "bytes_written": content.len(),
        }))
    }
}
