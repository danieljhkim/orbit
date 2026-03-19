use std::fs;
use std::path::Path;

use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct FsListTool;

impl Tool for FsListTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.list".to_string(),
            description: "List directory entries".to_string(),
            parameters: vec![ToolParam {
                name: "path".to_string(),
                description: "Path to the directory to list".to_string(),
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

        super::check_workspace_boundary(ctx, Path::new(path_str))?;

        let entries = fs::read_dir(path_str)
            .map_err(|e| OrbitError::Io(e.to_string()))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path().to_string_lossy().to_string())
            .collect::<Vec<_>>();

        Ok(json!({"path": path_str, "entries": entries}))
    }
}
