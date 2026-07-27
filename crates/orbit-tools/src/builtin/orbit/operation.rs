//! The MCP adapter over the operations-as-data kernel [ORB-10358].
//!
//! ADR-0209 bearing 1: an operation is declared once in `orbit-common` and each
//! surface derives its wiring. This module is the MCP half — it turns an
//! [`OperationSpec`] into the [`ToolSchema`] MCP advertises and into the
//! [`McpToolPolicy`] the registry enforces, so no verb needs a hand-written
//! [`Tool`](crate::Tool) impl.
//!
//! It is generic over the verb type on purpose: the next noun to migrate reuses
//! both functions unchanged and supplies only its own registry.

use orbit_common::operation::{McpExposure, OperationSpec};
use orbit_common::types::{McpToolPolicy, ToolParam, ToolSchema};

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
    match spec.mcp {
        McpExposure::Inactive => registry.register_inactive(tool),
        McpExposure::AgentOperator(placement) => {
            registry.register_mcp(tool, McpToolPolicy::agent_and_operator(placement));
        }
        McpExposure::OperatorOnly(placement) => {
            registry.register_mcp(tool, McpToolPolicy::operator_only(placement));
        }
    }
}
