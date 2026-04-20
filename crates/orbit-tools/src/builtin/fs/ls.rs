use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct FsLsTool;

impl Tool for FsLsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.ls".to_string(),
            description: "List entries in a directory; recurses up to `depth` levels (default 1, non-recursive)".to_string(),
            parameters: vec![
                ToolParam {
                    name: "path".to_string(),
                    description: "Path to the directory to list".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "depth".to_string(),
                    description: "Maximum recursion depth (>=1). 1 lists only immediate children.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
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

        let depth = match input.get("depth") {
            None | Some(Value::Null) => 1u32,
            Some(v) => {
                let n = v.as_u64().ok_or_else(|| {
                    OrbitError::InvalidInput("`depth` must be a positive integer".to_string())
                })?;
                if n < 1 {
                    return Err(OrbitError::InvalidInput("`depth` must be >= 1".to_string()));
                }
                u32::try_from(n)
                    .map_err(|_| OrbitError::InvalidInput("`depth` is too large".to_string()))?
            }
        };

        let canonical = super::check_workspace_boundary(ctx, Path::new(path_str))?;
        let policy = super::check_read_policy(ctx, &canonical)?;
        let metadata = fs::metadata(&canonical).map_err(|e| OrbitError::Io(e.to_string()))?;
        if !metadata.is_dir() {
            return Err(OrbitError::InvalidInput(format!(
                "path is not a directory: {}",
                canonical.display()
            )));
        }

        let mut entries = Vec::new();
        walk(&canonical, 1, depth, &mut entries)?;
        super::emit_success(ctx, policy.as_ref())?;

        Ok(json!({
            "path": canonical.display().to_string(),
            "depth": depth,
            "entries": entries,
        }))
    }
}

fn walk(
    dir: &Path,
    current_depth: u32,
    max_depth: u32,
    out: &mut Vec<Value>,
) -> Result<(), OrbitError> {
    let mut level: Vec<(PathBuf, std::fs::FileType, std::fs::Metadata)> = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| OrbitError::Io(e.to_string()))? {
        let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|e| OrbitError::Io(e.to_string()))?;
        let metadata = entry
            .metadata()
            .map_err(|e| OrbitError::Io(e.to_string()))?;
        level.push((entry.path(), file_type, metadata));
    }
    level.sort_by(|a, b| a.0.file_name().cmp(&b.0.file_name()));

    for (path, file_type, metadata) in level {
        let kind = if file_type.is_symlink() {
            "symlink"
        } else if file_type.is_dir() {
            "directory"
        } else if file_type.is_file() {
            "file"
        } else {
            "other"
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.push(json!({
            "name": name,
            "path": path.display().to_string(),
            "kind": kind,
            "bytes": metadata.len(),
            "depth": current_depth,
        }));

        // Recurse only into real directories (not symlinks, to avoid cycles).
        if file_type.is_dir() && current_depth < max_depth {
            walk(&path, current_depth + 1, max_depth, out)?;
        }
    }
    Ok(())
}
