use orbit_core::OrbitError;

use super::super::dispatch::ConfigTarget;
use super::common::ServerLaunch;
use super::simple_json::{apply_simple_json_init, apply_simple_json_remove};

pub(in crate::command::mcp::setup) fn apply_gemini_init(
    target: &ConfigTarget,
    launch: ServerLaunch<'_>,
) -> Result<(), OrbitError> {
    apply_simple_json_init(target, "mcpServers", launch)
}

pub(in crate::command::mcp::setup) fn apply_gemini_remove(
    target: &ConfigTarget,
    server_id: &str,
) -> Result<(), OrbitError> {
    apply_simple_json_remove(target, "mcpServers", server_id)
}
