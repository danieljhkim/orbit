//! MCP adapter over the operations-as-data kernel.
//!
//! An operation is declared once in `orbit-common` and each surface derives
//! its wiring. This module turns an [`OperationSpec`] into the [`ToolSchema`]
//! MCP advertises and registers it at the declared workspace scope, so no verb
//! needs a hand-written [`Tool`](crate::Tool) implementation.
//!
//! It is generic over the verb type on purpose: the next noun to migrate reuses
//! both functions unchanged and supplies only its own registry.

use orbit_common::operation::OperationSpec;
use orbit_common::types::{ToolParam, ToolSchema};

use crate::ToolRegistry;

/// Derive the MCP tool schema an operation advertises.
///
/// Parameter order follows the spec's declaration order, which is contract: it
/// is what the shipped `mcp_tools_list` snapshot records.
pub(super) fn operation_tool_schema<V: 'static>(spec: &OperationSpec<V>) -> ToolSchema {
    ToolSchema {
        name: spec.tool_name.to_string(),
        description: spec.tool_description.to_string(),
        parameters: spec
            .mcp_params()
            .map(|(param, description)| ToolParam {
                name: param.name.to_string(),
                description: description.resolve(),
                param_type: param.param_type.as_tool_param_type().to_string(),
                required: param.required,
            })
            .collect(),
        builtin: true,
    }
}

/// Register one derived tool under the exposure its spec declares.
pub(super) fn register_operation<V: 'static, T: crate::Tool + 'static>(
    registry: &mut ToolRegistry,
    spec: &OperationSpec<V>,
    tool: T,
) {
    match spec.mcp_scope {
        Some(scope) => registry.register_mcp(tool, scope),
        None => registry.register_inactive(tool),
    }
}
