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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use orbit_types::OrbitError;
    use serde_json::json;

    use crate::{Tool, ToolContext};

    use super::FsPatchTool;

    #[cfg(unix)]
    fn create_file_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(src, dst)
    }

    #[cfg(windows)]
    fn create_file_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(src, dst)
    }

    #[test]
    fn replaces_first_occurrence_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("file.txt");
        fs::write(&path, "hello world hello").expect("seed file");

        let result = FsPatchTool
            .execute(
                &ToolContext {
                    workspace_root: Some(dir.path().to_path_buf()),
                    ..Default::default()
                },
                json!({
                    "path": path.display().to_string(),
                    "old_string": "hello",
                    "new_string": "hi",
                }),
            )
            .expect("patch succeeds");

        assert_eq!(
            fs::read_to_string(&path).expect("read file"),
            "hi world hello"
        );
        let canonical_path = path.canonicalize().expect("canonical path");
        assert_eq!(result["path"], canonical_path.display().to_string());
        assert_eq!(result["replacements"], 1);
    }

    #[test]
    fn rejects_missing_old_string() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("file.txt");
        fs::write(&path, "hello world").expect("seed file");

        let err = FsPatchTool
            .execute(
                &ToolContext {
                    workspace_root: Some(dir.path().to_path_buf()),
                    ..Default::default()
                },
                json!({
                    "path": path.display().to_string(),
                    "old_string": "goodbye",
                    "new_string": "hi",
                }),
            )
            .expect_err("missing match should fail");

        assert!(matches!(err, OrbitError::InvalidInput(_)));
    }

    #[test]
    fn rejects_paths_that_escape_workspace() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        let target = outside.path().join("target.txt");
        let alias = workspace.path().join("alias.txt");
        fs::write(&target, "hello").expect("seed file");
        create_file_symlink(&target, &alias).expect("create symlink");

        let err = FsPatchTool
            .execute(
                &ToolContext {
                    workspace_root: Some(workspace.path().to_path_buf()),
                    ..Default::default()
                },
                json!({
                    "path": alias.display().to_string(),
                    "old_string": "hello",
                    "new_string": "hi",
                }),
            )
            .expect_err("workspace escape should be denied");

        assert!(matches!(err, OrbitError::PolicyDenied(_)));
    }
}
