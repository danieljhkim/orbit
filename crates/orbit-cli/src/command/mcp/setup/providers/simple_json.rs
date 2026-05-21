use orbit_core::OrbitError;
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::command::mcp::ORBIT_MCP_SERVER_ID;

use super::super::dispatch::ConfigTarget;
use super::super::format::*;
use super::common::server_args;

pub(in crate::command::mcp::setup) fn apply_simple_json_init(
    target: &ConfigTarget,
    top_level_key: &str,
) -> Result<(), OrbitError> {
    let mut root = load_json_object(&target.mcp_path)?;
    let servers = ensure_json_object(&mut root, top_level_key)?;
    servers.insert(ORBIT_MCP_SERVER_ID.to_string(), simple_mcp_server_value());
    write_json_object(&target.mcp_path, &root)
}

pub(in crate::command::mcp::setup) fn apply_simple_json_remove(
    target: &ConfigTarget,
    top_level_key: &str,
) -> Result<(), OrbitError> {
    let mut root = load_json_object(&target.mcp_path)?;
    if let Some(servers) = root
        .get_mut(top_level_key)
        .and_then(JsonValue::as_object_mut)
    {
        servers.remove(ORBIT_MCP_SERVER_ID);
        if servers.is_empty() {
            root.remove(top_level_key);
        }
    }
    write_or_remove_json_object(&target.mcp_path, &root)
}

pub(super) fn simple_mcp_server_value() -> JsonValue {
    JsonValue::Object(JsonMap::from_iter([
        (
            "command".to_string(),
            JsonValue::String("orbit".to_string()),
        ),
        (
            "args".to_string(),
            JsonValue::Array(server_args().into_iter().map(JsonValue::String).collect()),
        ),
    ]))
}
