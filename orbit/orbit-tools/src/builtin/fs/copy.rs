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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use orbit_types::OrbitError;
    use serde_json::json;

    use crate::{Tool, ToolContext};

    use super::FsCopyTool;

    #[cfg(unix)]
    fn create_file_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(src, dst)
    }

    #[cfg(windows)]
    fn create_file_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(src, dst)
    }

    #[test]
    fn copies_file_to_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("destination.txt");
        fs::write(&source, "hello").expect("seed source");

        let result = FsCopyTool
            .execute(
                &ToolContext {
                    workspace_root: Some(dir.path().to_path_buf()),
                    ..Default::default()
                },
                json!({
                    "source": source.display().to_string(),
                    "destination": destination.display().to_string(),
                }),
            )
            .expect("copy succeeds");

        assert_eq!(
            fs::read_to_string(&destination).expect("read destination"),
            "hello"
        );
        let canonical_root = dir.path().canonicalize().expect("canonical root");
        assert_eq!(
            result["source"],
            canonical_root.join("source.txt").display().to_string()
        );
        assert_eq!(
            result["destination"],
            canonical_root.join("destination.txt").display().to_string()
        );
        assert_eq!(result["bytes_copied"], 5);
    }

    #[test]
    fn rejects_source_outside_workspace() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        let source = outside.path().join("source.txt");
        let destination = workspace.path().join("destination.txt");
        fs::write(&source, "hello").expect("seed source");

        let err = FsCopyTool
            .execute(
                &ToolContext {
                    workspace_root: Some(workspace.path().to_path_buf()),
                    ..Default::default()
                },
                json!({
                    "source": source.display().to_string(),
                    "destination": destination.display().to_string(),
                }),
            )
            .expect_err("outside source should be denied");

        assert!(matches!(err, OrbitError::PolicyDenied(_)));
    }

    #[test]
    fn rejects_destination_symlink_that_escapes_workspace() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        let source = workspace.path().join("source.txt");
        let target = outside.path().join("target.txt");
        let alias = workspace.path().join("alias.txt");
        fs::write(&source, "hello").expect("seed source");
        fs::write(&target, "outside").expect("seed target");
        create_file_symlink(&target, &alias).expect("create symlink");

        let err = FsCopyTool
            .execute(
                &ToolContext {
                    workspace_root: Some(workspace.path().to_path_buf()),
                    ..Default::default()
                },
                json!({
                    "source": source.display().to_string(),
                    "destination": alias.display().to_string(),
                }),
            )
            .expect_err("workspace escape should be denied");

        assert!(matches!(err, OrbitError::PolicyDenied(_)));
    }
}
