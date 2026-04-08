use std::fs;
use std::path::Path;

use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct FsDeleteTool;

impl Tool for FsDeleteTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.delete".to_string(),
            description: "Delete a file from disk".to_string(),
            parameters: vec![ToolParam {
                name: "path".to_string(),
                description: "Path to the file to delete".to_string(),
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
        super::check_file_lock(ctx, &canonical)?;

        fs::remove_file(&canonical).map_err(|e| OrbitError::Io(e.to_string()))?;

        Ok(json!({"path": canonical.display().to_string(), "deleted": true}))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use serde_json::json;

    use crate::{Tool, ToolContext};

    use super::FsDeleteTool;

    #[cfg(unix)]
    fn create_file_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(src, dst)
    }

    #[cfg(windows)]
    fn create_file_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(src, dst)
    }

    #[test]
    fn deletes_canonical_target_for_symlink_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target.txt");
        let alias = dir.path().join("alias.txt");
        fs::write(&target, "before").expect("seed target");
        create_file_symlink(&target, &alias).expect("create symlink");

        let result = FsDeleteTool
            .execute(
                &ToolContext {
                    workspace_root: Some(dir.path().to_path_buf()),
                    ..Default::default()
                },
                json!({
                    "path": alias.display().to_string(),
                }),
            )
            .expect("delete succeeds");

        assert!(!target.exists());
        // Canonicalize before deletion to handle macOS /var -> /private/var
        let canonical_target = dir
            .path()
            .canonicalize()
            .expect("canonical dir")
            .join("target.txt");
        assert_eq!(result["path"], canonical_target.display().to_string());
        assert_eq!(result["deleted"], true);
    }
}
