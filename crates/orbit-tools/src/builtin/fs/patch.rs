use std::fs;
use std::path::Path;

use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct FsPatchTool;

impl Tool for FsPatchTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.patch".to_string(),
            description: "Replace the first occurrence of a string in a UTF-8 file".to_string(),
            parameters: vec![
                ToolParam {
                    name: "path".to_string(),
                    description: "Path to the file to modify".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "old_string".to_string(),
                    description: "The string to replace".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "new_string".to_string(),
                    description: "The replacement string".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let path = input
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| OrbitError::InvalidInput("missing `path`".to_string()))?;
        let old_string = input
            .get("old_string")
            .and_then(Value::as_str)
            .ok_or_else(|| OrbitError::InvalidInput("missing `old_string`".to_string()))?;
        let new_string = input
            .get("new_string")
            .and_then(Value::as_str)
            .ok_or_else(|| OrbitError::InvalidInput("missing `new_string`".to_string()))?;

        if old_string.is_empty() {
            return Err(OrbitError::InvalidInput(
                "`old_string` must not be empty".to_string(),
            ));
        }

        let canonical = super::check_workspace_boundary(ctx, Path::new(path))?;
        super::check_file_lock(ctx, &canonical)?;

        let content = fs::read_to_string(&canonical).map_err(|e| OrbitError::Io(e.to_string()))?;
        if !content.contains(old_string) {
            return Err(OrbitError::InvalidInput(format!(
                "`old_string` not found in file: {}",
                canonical.display()
            )));
        }

        let updated = content.replacen(old_string, new_string, 1);
        fs::write(&canonical, updated).map_err(|e| OrbitError::Io(e.to_string()))?;

        Ok(json!({
            "path": canonical.display().to_string(),
            "replacements": 1,
        }))
    }
}
