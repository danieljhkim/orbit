use serde_json::{Value, json};

/// Keep MCP `structuredContent` object-shaped for clients that enforce record
/// results (notably Cursor and VS Code), while preserving non-object payloads.
pub(super) fn mcp_structured_content(value: Value) -> Value {
    match value {
        Value::Object(_) => value,
        Value::Array(items) => json!({ "items": items }),
        value => json!({ "value": value }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mcp_structured_content_preserves_existing_objects() {
        let value = json!({ "ok": true });
        assert_eq!(mcp_structured_content(value.clone()), value);
    }
}
