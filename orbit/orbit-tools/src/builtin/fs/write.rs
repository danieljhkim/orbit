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

        let canonical = super::check_workspace_boundary(ctx, Path::new(path_str))?;
        super::check_file_lock(ctx, &canonical)?;

        fs::write(&canonical, content).map_err(|e| OrbitError::Io(e.to_string()))?;

        Ok(json!({
            "path": canonical.display().to_string(),
            "bytes_written": content.len(),
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

    use super::FsWriteTool;

    #[cfg(unix)]
    fn create_file_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(src, dst)
    }

    #[cfg(windows)]
    fn create_file_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(src, dst)
    }

    #[test]
    fn writes_to_canonical_target_for_symlink_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target.txt");
        let alias = dir.path().join("alias.txt");
        fs::write(&target, "before").expect("seed target");
        create_file_symlink(&target, &alias).expect("create symlink");

        let result = FsWriteTool
            .execute(
                &ToolContext {
                    workspace_root: Some(dir.path().to_path_buf()),
                    ..Default::default()
                },
                json!({
                    "path": alias.display().to_string(),
                    "content": "after",
                }),
            )
            .expect("write succeeds");

        assert_eq!(fs::read_to_string(&target).expect("read target"), "after");
        let canonical_target = target.canonicalize().expect("canonical target");
        assert_eq!(result["path"], canonical_target.display().to_string());
        assert_eq!(result["bytes_written"], 5);
    }

    #[test]
    fn rejects_symlink_paths_that_escape_workspace() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        let target = outside.path().join("target.txt");
        let alias = workspace.path().join("alias.txt");
        fs::write(&target, "outside").expect("seed target");
        create_file_symlink(&target, &alias).expect("create symlink");

        let err = FsWriteTool
            .execute(
                &ToolContext {
                    workspace_root: Some(workspace.path().to_path_buf()),
                    ..Default::default()
                },
                json!({
                    "path": alias.display().to_string(),
                    "content": "after",
                }),
            )
            .expect_err("workspace escape should be denied");

        assert!(matches!(err, OrbitError::PolicyDenied(_)));
    }
}
