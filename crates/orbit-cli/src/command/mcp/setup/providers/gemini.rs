use orbit_core::OrbitError;

use super::super::dispatch::ConfigTarget;
use super::simple_json::{apply_simple_json_init, apply_simple_json_remove};

pub(in crate::command::mcp::setup) fn apply_gemini_init(
    target: &ConfigTarget,
) -> Result<(), OrbitError> {
    apply_simple_json_init(target, "mcpServers")
}

pub(in crate::command::mcp::setup) fn apply_gemini_remove(
    target: &ConfigTarget,
) -> Result<(), OrbitError> {
    apply_simple_json_remove(target, "mcpServers")
}

