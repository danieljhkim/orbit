use super::*;
use serde_json::json;

#[test]
fn mcp_structured_content_preserves_existing_objects() {
    let value = json!({ "ok": true });
    assert_eq!(mcp_structured_content(value.clone()), value);
}
