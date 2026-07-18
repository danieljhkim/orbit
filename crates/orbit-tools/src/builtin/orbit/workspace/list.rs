use orbit_common::types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

/// Canonical operator discovery tool: enumerate sanitized workspace ownership
/// and execution-profile freshness. Hub placement, operator capability,
/// workspace-unscoped. The response carries only stable workspace identity,
/// declared owner identity/display name, and missing/current/stale profile
/// metadata (generation and age) — never a checkout path, raw profile payload,
/// crew, or model.
pub struct OrbitWorkspaceListTool;

impl Tool for OrbitWorkspaceListTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.workspace.list".to_string(),
            description: "List workspaces with declared owner and sanitized execution-profile \
                          freshness (operator, hub placement)."
                .to_string(),
            // Workspace-unscoped: no `workspace` parameter and no workspace
            // resolution — this is a global registry projection.
            parameters: Vec::new(),
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::WorkspaceList)
    }
}
