use serde_json::json;

use super::proc_spawn_timeout_ms;
use crate::TIMEOUT_DEFAULT_MS;

#[test]
fn missing_proc_spawn_timeout_uses_default_timeout() {
    assert_eq!(proc_spawn_timeout_ms(&json!({})), TIMEOUT_DEFAULT_MS);
}

#[test]
fn explicit_proc_spawn_timeout_is_preserved() {
    assert_eq!(proc_spawn_timeout_ms(&json!({ "timeout_ms": 42 })), 42);
}
