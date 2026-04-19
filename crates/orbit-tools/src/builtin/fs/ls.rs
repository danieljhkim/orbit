use std::fs;
use std::path::Path;

use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct FsLsTool;

impl Tool for FsLsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.ls".to_string(),
            description: "List the entries in a directory".to_string(),
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

        let canonical = super::check_workspace_boundary(ctx, Path::new(path_str))?;
        let metadata = fs::metadata(&canonical).map_err(|e| OrbitError::Io(e.to_string()))?;
        if !metadata.is_dir() {
            return Err(OrbitError::InvalidInput(format!(
                "path is not a directory: {}",
                canonical.display()
            )));
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(&canonical).map_err(|e| OrbitError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let path = entry.path();
            let metadata = entry
                .metadata()
                .map_err(|e| OrbitError::Io(e.to_string()))?;
            let kind = if metadata.is_dir() {
                "directory"
            } else if metadata.is_file() {
                "file"
            } else {
                "other"
            };
            entries.push(json!({
                "name": entry.file_name().to_string_lossy().into_owned(),
                "path": path.display().to_string(),
                "kind": kind,
                "bytes": metadata.len(),
            }));
        }

        entries.sort_by(|left, right| {
            let left_name = left.get("name").and_then(Value::as_str).unwrap_or_default();
            let right_name = right
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            left_name.cmp(right_name)
        });

        Ok(json!({
            "path": canonical.display().to_string(),
            "entries": entries,
        }))
    }
}
