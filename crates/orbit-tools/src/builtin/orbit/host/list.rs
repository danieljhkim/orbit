use orbit_common::types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

/// Canonical operator discovery tool: enumerate the sanitized hub host
/// registry. Hub placement, operator capability, workspace-unscoped. The
/// response carries only stable machine identity, display name, labels,
/// lifecycle/liveness, permanent aliases, and workspace-presence freshness —
/// never a presence root, checkout path, or any credential.
pub struct OrbitHostListTool;

impl Tool for OrbitHostListTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.host.list".to_string(),
            description: "List registered hub hosts with sanitized lifecycle, aliases, and \
                          workspace-presence freshness (operator, hub placement)."
                .to_string(),
            // Workspace-unscoped: deliberately no `workspace` parameter and no
            // workspace resolution — this is a global registry projection.
            parameters: Vec::new(),
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::HostList)
    }
}
