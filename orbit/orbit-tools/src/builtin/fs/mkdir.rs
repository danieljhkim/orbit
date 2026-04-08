use std::fs;
use std::path::Path;

use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct FsMkdirTool;

impl Tool for FsMkdirTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.mkdir".to_string(),
            description: "Create a directory and any missing parent directories".to_string(),
            parameters: vec![ToolParam {
                name: "path".to_string(),
                description: "Path to the directory to create".to_string(),
                param_type: "string".to_string(),
                required: true,
            }],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let path = input
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| OrbitError::InvalidInput("missing `path`".to_string()))?;

        let canonical = super::check_workspace_boundary(ctx, Path::new(path))?;
        fs::create_dir_all(&canonical).map_err(|e| OrbitError::Io(e.to_string()))?;

        Ok(json!({
            "path": canonical.display().to_string(),
            "created": true,
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

    use super::FsMkdirTool;

    #[cfg(unix)]
    fn create_dir_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(src, dst)
    }

    #[cfg(windows)]
    fn create_dir_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(src, dst)
    }

    #[test]
    fn creates_nested_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a/b/c");

        let result = FsMkdirTool
            .execute(
                &ToolContext {
                    workspace_root: Some(dir.path().to_path_buf()),
                    ..Default::default()
                },
                json!({
                    "path": path.display().to_string(),
                }),
            )
            .expect("mkdir succeeds");

        assert!(path.is_dir());
        let canonical_root = dir.path().canonicalize().expect("canonical root");
        assert_eq!(
            result["path"],
            canonical_root.join("a/b/c").display().to_string()
        );
        assert_eq!(result["created"], true);
    }

    #[test]
    fn is_noop_when_directory_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("existing");
        fs::create_dir_all(&path).expect("seed dir");

        let result = FsMkdirTool
            .execute(
                &ToolContext {
                    workspace_root: Some(dir.path().to_path_buf()),
                    ..Default::default()
                },
                json!({
                    "path": path.display().to_string(),
                }),
            )
            .expect("mkdir succeeds");

        assert!(path.is_dir());
        assert_eq!(result["created"], true);
    }

    #[test]
    fn rejects_paths_that_escape_workspace() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        let alias = workspace.path().join("alias");
        create_dir_symlink(outside.path(), &alias).expect("create symlink");

        let err = FsMkdirTool
            .execute(
                &ToolContext {
                    workspace_root: Some(workspace.path().to_path_buf()),
                    ..Default::default()
                },
                json!({
                    "path": alias.join("child").display().to_string(),
                }),
            )
            .expect_err("workspace escape should be denied");

        assert!(matches!(err, OrbitError::PolicyDenied(_)));
    }
}
