use std::fs;
use std::path::Path;

use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct FsMoveTool;

impl Tool for FsMoveTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.move".to_string(),
            description: "Move or rename a file".to_string(),
            parameters: vec![
                ToolParam {
                    name: "source".to_string(),
                    description: "Path to the source file".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "destination".to_string(),
                    description: "Path to move the file to".to_string(),
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
        super::check_file_lock(ctx, &canonical_source)?;
        super::check_file_lock(ctx, &canonical_destination)?;

        fs::rename(&canonical_source, &canonical_destination)
            .map_err(|e| OrbitError::Io(e.to_string()))?;

        Ok(json!({
            "source": canonical_source.display().to_string(),
            "destination": canonical_destination.display().to_string(),
            "moved": true,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use orbit_lock::{FileLockChecker, FileLockStore, apply_lock_schema};
    use orbit_types::OrbitError;
    use rusqlite::Connection;
    use serde_json::json;

    use crate::{Tool, ToolContext};

    use super::FsMoveTool;

    #[cfg(unix)]
    fn create_file_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(src, dst)
    }

    #[cfg(windows)]
    fn create_file_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(src, dst)
    }

    fn lock_store() -> Arc<FileLockStore> {
        let conn = Connection::open_in_memory().expect("sqlite");
        apply_lock_schema(&conn).expect("schema");
        Arc::new(FileLockStore::new(Arc::new(Mutex::new(conn))))
    }

    #[test]
    fn moves_file_to_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("destination.txt");
        fs::write(&source, "hello").expect("seed source");

        let result = FsMoveTool
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
            .expect("move succeeds");

        assert!(!source.exists());
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
        assert_eq!(result["moved"], true);
    }

    #[test]
    fn rejects_source_outside_workspace() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        let source = outside.path().join("source.txt");
        let destination = workspace.path().join("destination.txt");
        fs::write(&source, "hello").expect("seed source");

        let err = FsMoveTool
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

        let err = FsMoveTool
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

    #[test]
    fn checks_file_locks_for_source_and_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("destination.txt");
        fs::write(&source, "hello").expect("seed source");
        let store = lock_store();
        let repo_root = dir.path().canonicalize().expect("canonical root");
        let repo_root_str = repo_root.to_string_lossy().into_owned();
        store
            .auto_acquire("T-source", &repo_root_str, "source.txt")
            .expect("lock source");

        let err = FsMoveTool
            .execute(
                &ToolContext {
                    workspace_root: Some(dir.path().to_path_buf()),
                    task_id: Some("T-destination".to_string()),
                    file_lock_checker: Some(store.clone()),
                    ..Default::default()
                },
                json!({
                    "source": source.display().to_string(),
                    "destination": destination.display().to_string(),
                }),
            )
            .expect_err("source lock should be enforced");
        assert!(matches!(err, OrbitError::PolicyDenied(_)));

        store
            .release_locks_for_task("T-source")
            .expect("release source lock");
        store
            .auto_acquire("T-other", &repo_root_str, "destination.txt")
            .expect("lock destination");

        let err = FsMoveTool
            .execute(
                &ToolContext {
                    workspace_root: Some(dir.path().to_path_buf()),
                    task_id: Some("T-destination".to_string()),
                    file_lock_checker: Some(store),
                    ..Default::default()
                },
                json!({
                    "source": source.display().to_string(),
                    "destination": destination.display().to_string(),
                }),
            )
            .expect_err("destination lock should be enforced");
        assert!(matches!(err, OrbitError::PolicyDenied(_)));
    }
}
