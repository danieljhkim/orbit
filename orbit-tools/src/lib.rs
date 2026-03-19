pub mod builtin;
pub mod external;
pub mod registry;

use std::path::PathBuf;

use serde_json::{Map, Value};

use orbit_types::{OrbitError, ToolSchema};

pub use registry::ToolRegistry;

#[derive(Debug, Clone, Default)]
pub struct ToolContext {
    pub cwd: Option<String>,
    /// When set, fs tools enforce that all paths resolve inside this directory.
    /// Symlink escapes are blocked because paths are canonicalized before the check.
    /// If `None`, no boundary is enforced (used for tests and legacy callers).
    pub workspace_root: Option<PathBuf>,
}

pub trait Tool: Send + Sync {
    fn schema(&self) -> ToolSchema;
    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError>;
}

pub fn map_input_from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Value {
    let mut map = Map::new();
    for (key, value) in pairs {
        map.insert(key, Value::String(value));
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::{ToolContext, ToolRegistry};

    #[test]
    fn registry_contains_expected_builtins() {
        let mut registry = ToolRegistry::new();
        registry.register_builtins();
        let names = registry
            .schemas()
            .into_iter()
            .map(|s| s.name)
            .collect::<Vec<_>>();

        assert!(names.contains(&"fs.read".to_string()));
        assert!(names.contains(&"fs.write".to_string()));
        assert!(names.contains(&"git.stage_paths".to_string()));
        assert!(names.contains(&"git.commit".to_string()));
        assert!(names.contains(&"proc.spawn".to_string()));
        assert!(names.contains(&"time.now".to_string()));
        assert!(names.contains(&"github.auth.status".to_string()));
        assert!(names.contains(&"github.pr.create".to_string()));
    }

    #[test]
    fn fs_read_returns_file_contents() {
        let dir = tempdir().expect("temp dir");
        let path = dir.path().join("note.txt");
        fs::write(&path, "hello").expect("write file");

        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let output = registry
            .execute(
                "fs.read",
                &ToolContext::default(),
                json!({"path": path.to_string_lossy()}),
            )
            .expect("tool executes");

        assert_eq!(output["content"], "hello");
    }

    #[test]
    fn fs_tools_inside_workspace_succeed() {
        let workspace = tempdir().expect("workspace dir");
        let file = workspace.path().join("data.txt");
        fs::write(&file, "contents").expect("write file");

        let ctx = ToolContext {
            workspace_root: Some(workspace.path().canonicalize().expect("canonicalize")),
            ..Default::default()
        };
        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let output = registry
            .execute("fs.read", &ctx, json!({"path": file.to_string_lossy()}))
            .expect("read inside workspace should succeed");
        assert_eq!(output["content"], "contents");

        registry
            .execute(
                "fs.list",
                &ctx,
                json!({"path": workspace.path().to_string_lossy()}),
            )
            .expect("list inside workspace should succeed");

        let write_target = workspace.path().join("out.txt");
        registry
            .execute(
                "fs.write",
                &ctx,
                json!({"path": write_target.to_string_lossy(), "content": "ok"}),
            )
            .expect("write inside workspace should succeed");

        registry
            .execute(
                "fs.delete",
                &ctx,
                json!({"path": write_target.to_string_lossy()}),
            )
            .expect("delete inside workspace should succeed");
    }

    #[test]
    fn fs_read_outside_workspace_is_denied() {
        let workspace = tempdir().expect("workspace dir");
        let outside = tempdir().expect("outside dir");
        let outside_file = outside.path().join("secret.txt");
        fs::write(&outside_file, "secret").expect("write outside file");

        let ctx = ToolContext {
            workspace_root: Some(workspace.path().canonicalize().expect("canonicalize")),
            ..Default::default()
        };
        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let err = registry
            .execute(
                "fs.read",
                &ctx,
                json!({"path": outside_file.to_string_lossy()}),
            )
            .expect_err("read outside workspace must be denied");
        assert!(
            err.to_string().contains("outside workspace"),
            "expected policy denied message, got: {err}"
        );
    }

    #[test]
    fn fs_write_outside_workspace_is_denied() {
        let workspace = tempdir().expect("workspace dir");
        let outside = tempdir().expect("outside dir");
        let outside_target = outside.path().join("injected.txt");

        let ctx = ToolContext {
            workspace_root: Some(workspace.path().canonicalize().expect("canonicalize")),
            ..Default::default()
        };
        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let err = registry
            .execute(
                "fs.write",
                &ctx,
                json!({"path": outside_target.to_string_lossy(), "content": "pwned"}),
            )
            .expect_err("write outside workspace must be denied");
        assert!(
            err.to_string().contains("outside workspace"),
            "expected policy denied message, got: {err}"
        );
    }
}
