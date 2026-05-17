use std::collections::{BTreeMap, HashMap};
use std::fmt;

use orbit_common::types::ToolSchema;
use rmcp::ErrorData as McpError;
use serde_json::json;

/// Sanitize an Orbit tool name into the character set MCP clients accept.
///
/// Cursor enforces `[a-zA-Z0-9_]` and VS Code enforces `[a-z0-9_-]`. Replacing
/// `.` with `_` keeps Orbit's existing names within the intersection of both
/// rule sets without renaming any internal canonical identifier.
pub(super) fn sanitize_tool_name(name: &str) -> String {
    name.replace('.', "_")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ToolNameCollision {
    pub(super) advertised_name: String,
    pub(super) canonical_names: Vec<String>,
}

impl ToolNameCollision {
    pub(super) fn into_mcp_error(self) -> McpError {
        let message = self.to_string();
        McpError::internal_error(
            message,
            Some(json!({
                "code": "tool_name_collision",
                "advertised_name": self.advertised_name,
                "canonical_names": self.canonical_names,
            })),
        )
    }
}

impl fmt::Display for ToolNameCollision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "MCP tool name collision: advertised name '{}' is produced by canonical tools {}; rename one tool before exposing over MCP",
            self.advertised_name,
            self.canonical_names.join(", ")
        )
    }
}

pub(super) fn build_name_map(
    schemas: &[ToolSchema],
) -> Result<HashMap<String, String>, ToolNameCollision> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for schema in schemas {
        grouped
            .entry(sanitize_tool_name(&schema.name))
            .or_default()
            .push(schema.name.clone());
    }

    let mut map = HashMap::with_capacity(schemas.len());
    for (advertised_name, mut canonical_names) in grouped {
        canonical_names.sort();
        canonical_names.dedup();
        if canonical_names.len() > 1 {
            return Err(ToolNameCollision {
                advertised_name,
                canonical_names,
            });
        }
        if let Some(canonical_name) = canonical_names.pop() {
            map.insert(advertised_name, canonical_name);
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    use super::super::test_support::tool_schema;

    #[test]
    fn sanitize_tool_name_replaces_dots_with_underscores() {
        assert_eq!(sanitize_tool_name("orbit.task.add"), "orbit_task_add");
        assert_eq!(
            sanitize_tool_name("orbit.task.review_thread.add"),
            "orbit_task_review_thread_add"
        );
        assert_eq!(sanitize_tool_name("orbit_task_add"), "orbit_task_add");
    }

    #[test]
    fn build_name_map_keys_are_advertised_names() {
        let schemas = vec![
            tool_schema("orbit.task.add"),
            tool_schema("orbit.task.review_thread.add"),
        ];
        let map = build_name_map(&schemas).expect("unique advertised names");
        assert_eq!(
            map.get("orbit_task_add").map(String::as_str),
            Some("orbit.task.add")
        );
        assert_eq!(
            map.get("orbit_task_review_thread_add").map(String::as_str),
            Some("orbit.task.review_thread.add")
        );
    }

    #[test]
    fn build_name_map_rejects_sanitized_name_collisions() {
        let schemas = vec![tool_schema("foo.bar"), tool_schema("foo_bar")];
        let err = build_name_map(&schemas).expect_err("sanitized names must be unique");
        assert_eq!(err.advertised_name, "foo_bar");
        assert_eq!(
            err.canonical_names,
            vec!["foo.bar".to_string(), "foo_bar".to_string()]
        );

        let mcp_err = err.into_mcp_error();
        assert!(mcp_err.message.contains("foo_bar"));
        let data = mcp_err.data.as_ref().expect("structured error data");
        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some("tool_name_collision")
        );
        assert_eq!(
            data.get("advertised_name").and_then(Value::as_str),
            Some("foo_bar")
        );
    }
}
