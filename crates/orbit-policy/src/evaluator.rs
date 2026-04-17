use std::collections::HashSet;

use crate::PolicyDecision;
use crate::engine::PolicyContext;
use orbit_types::Role;

pub(crate) fn evaluate(
    ctx: &PolicyContext,
    denied_tools: &HashSet<String>,
    default_allow: bool,
) -> PolicyDecision {
    if let Some(tool_name) = &ctx.tool_name
        && ctx.role != Role::Admin
        && denied_tools.contains(tool_name)
    {
        return PolicyDecision::Deny {
            reason: format!("tool `{tool_name}` denied by policy"),
        };
    }

    if default_allow {
        PolicyDecision::Allow
    } else {
        PolicyDecision::Deny {
            reason: "default deny policy".to_string(),
        }
    }
}
