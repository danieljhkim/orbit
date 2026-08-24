use serde_json::json;

use std::path::Path;

use super::{path_argument, proc_spawn_timeout_ms};
use crate::TIMEOUT_DEFAULT_MS;

#[test]
fn missing_proc_spawn_timeout_uses_default_timeout() {
    assert_eq!(proc_spawn_timeout_ms(&json!({})), TIMEOUT_DEFAULT_MS);
}

#[test]
fn explicit_proc_spawn_timeout_is_preserved() {
    assert_eq!(proc_spawn_timeout_ms(&json!({ "timeout_ms": 42 })), 42);
}

#[test]
fn option_value_paths_are_recognized_without_treating_flags_as_paths() {
    let cwd = Path::new("/workspace");
    assert_eq!(
        path_argument("--file=./secret.txt", cwd),
        Some(cwd.join("./secret.txt"))
    );
    assert_eq!(path_argument("--verbose", cwd), None);
}
