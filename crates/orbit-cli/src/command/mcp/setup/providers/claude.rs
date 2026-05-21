use orbit_core::OrbitError;
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::command::mcp::{ORBIT_MCP_SERVER_ID, safe_mcp_tool_names};

use super::super::dispatch::ConfigTarget;
use super::super::format::*;
use super::common::server_args;

pub(in crate::command::mcp::setup) fn apply_claude_init(
    target: &ConfigTarget,
) -> Result<(), OrbitError> {
    let mut root = load_json_object(&target.mcp_path)?;
    let mcp_servers = ensure_json_object(&mut root, "mcpServers")?;
    mcp_servers.insert(ORBIT_MCP_SERVER_ID.to_string(), claude_mcp_server_value());
    write_json_object(&target.mcp_path, &root)?;

    if let Some(settings_path) = &target.settings_path {
        let mut settings = load_json_object(settings_path)?;
        let permissions = ensure_json_object(&mut settings, "permissions")?;
        let allow = ensure_json_string_array(permissions, "allow")?;
        merge_unique_strings(allow, claude_safe_permissions());
        write_json_object(settings_path, &settings)?;
    }
    Ok(())
}

pub(in crate::command::mcp::setup) fn apply_claude_remove(
    target: &ConfigTarget,
) -> Result<(), OrbitError> {
    let mut root = load_json_object(&target.mcp_path)?;
    if let Some(mcp_servers) = root
        .get_mut("mcpServers")
        .and_then(JsonValue::as_object_mut)
    {
        mcp_servers.remove(ORBIT_MCP_SERVER_ID);
        if mcp_servers.is_empty() {
            root.remove("mcpServers");
        }
    }
    write_or_remove_json_object(&target.mcp_path, &root)?;

    if let Some(settings_path) = &target.settings_path {
        let mut settings = load_json_object(settings_path)?;
        let mut remove_keys = Vec::new();
        if let Some(permissions) = settings
            .get_mut("permissions")
            .and_then(JsonValue::as_object_mut)
        {
            if let Some(allow) = permissions
                .get_mut("allow")
                .and_then(JsonValue::as_array_mut)
            {
                remove_known_strings(allow, &claude_safe_permissions());
                if allow.is_empty() {
                    permissions.remove("allow");
                }
            }
            if permissions.is_empty() {
                remove_keys.push("permissions".to_string());
            }
        }
        for key in remove_keys {
            settings.remove(&key);
        }
        write_or_remove_json_object(settings_path, &settings)?;
    }
    Ok(())
}

pub(super) fn claude_mcp_server_value() -> JsonValue {
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

fn claude_safe_permissions() -> Vec<String> {
    safe_mcp_tool_names()
        .into_iter()
        .map(claude_permission_name)
        .collect()
}

pub(super) fn claude_permission_name(tool_name: &str) -> String {
    // pub(super) widened so providers/tests/claude.rs can call it (sibling under providers per ORB-00221 layout)
    format!("mcp__plugin_orbit_orbit__{}", tool_name.replace('.', "_"))
}


