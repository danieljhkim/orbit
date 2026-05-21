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
mod tests;
