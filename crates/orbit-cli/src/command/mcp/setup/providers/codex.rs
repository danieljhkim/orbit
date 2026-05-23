use orbit_core::OrbitError;
use toml::{Table as TomlTable, Value as TomlValue};

use crate::command::mcp::ORBIT_MCP_SERVER_ID;

use super::super::dispatch::ConfigTarget;
use super::super::format::*;
use super::common::server_args;

pub(in crate::command::mcp::setup) fn apply_codex_init(
    target: &ConfigTarget,
) -> Result<(), OrbitError> {
    let mut root = load_toml_table(&target.mcp_path)?;
    let mcp_servers = ensure_toml_table(&mut root, "mcp_servers")?;
    mcp_servers.insert(
        ORBIT_MCP_SERVER_ID.to_string(),
        TomlValue::Table(codex_mcp_server_table()),
    );
    write_toml_table(&target.mcp_path, &root)
}

pub(in crate::command::mcp::setup) fn apply_codex_remove(
    target: &ConfigTarget,
) -> Result<(), OrbitError> {
    let mut root = load_toml_table(&target.mcp_path)?;
    if let Some(mcp_servers) = root
        .get_mut("mcp_servers")
        .and_then(TomlValue::as_table_mut)
    {
        mcp_servers.remove(ORBIT_MCP_SERVER_ID);
        if mcp_servers.is_empty() {
            root.remove("mcp_servers");
        }
    }
    write_or_remove_toml_table(&target.mcp_path, &root)
}

pub(super) fn codex_mcp_server_table() -> TomlTable {
    TomlTable::from_iter([
        (
            "command".to_string(),
            TomlValue::String("orbit".to_string()),
        ),
        (
            "args".to_string(),
            TomlValue::Array(server_args().into_iter().map(TomlValue::String).collect()),
        ),
        ("enabled".to_string(), TomlValue::Boolean(true)),
    ])
}
