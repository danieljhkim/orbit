pub mod builtin;
pub mod external;
pub mod registry;

use serde_json::{Map, Value};

use orbit_types::{OrbitError, ToolSchema};

pub use registry::ToolRegistry;

#[derive(Debug, Clone, Default)]
pub struct ToolContext {
    pub cwd: Option<String>,
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
}
