use std::fs;
use std::path::Path;

use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct FsCopyTool;

impl Tool for FsCopyTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.copy".to_string(),
            description: "Copy a file to a new path".to_string(),
            parameters: vec![
                ToolParam {
                    name: "source".to_string(),
                    description: "Path to the source file".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "destination".to_string(),
                    description: "Path to copy the file to".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let source = input
            .get("source")
            .and_then(Value::as_str)
            .ok_or_else(|| OrbitError::InvalidInput("missing `source`".to_string()))?;
        let destination = input
            .get("destination")
            .and_then(Value::as_str)
            .ok_or_else(|| OrbitError::InvalidInput("missing `destination`".to_string()))?;

        let canonical_source = super::check_workspace_boundary(ctx, Path::new(source))?;
        let canonical_destination = super::check_workspace_boundary(ctx, Path::new(destination))?;
        super::check_file_lock(ctx, &canonical_destination)?;

        let bytes_copied = fs::copy(&canonical_source, &canonical_destination)
            .map_err(|e| OrbitError::Io(e.to_string()))?;

        Ok(json!({
            "source": canonical_source.display().to_string(),
            "destination": canonical_destination.display().to_string(),
            "bytes_copied": bytes_copied,
        }))
    }
}
