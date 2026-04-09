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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use orbit_types::Role;

    use super::evaluate;
    use crate::{PolicyDecision, engine::PolicyContext};

    #[test]
    fn agent_denied_tool_is_rejected() {
        let denied_tools = HashSet::from([String::from("dangerous.tool")]);
        let decision = evaluate(
            &PolicyContext {
                entrypoint: "cli".to_string(),
                tool_name: Some("dangerous.tool".to_string()),
                role: Role::Agent,
            },
            &denied_tools,
            true,
        );

        assert_eq!(
            decision,
            PolicyDecision::Deny {
                reason: "tool `dangerous.tool` denied by policy".to_string(),
            }
        );
    }

    #[test]
    fn admin_bypasses_denied_tool() {
        let denied_tools = HashSet::from([String::from("dangerous.tool")]);
        let decision = evaluate(
            &PolicyContext {
                entrypoint: "cli".to_string(),
                tool_name: Some("dangerous.tool".to_string()),
                role: Role::Admin,
            },
            &denied_tools,
            true,
        );

        assert_eq!(decision, PolicyDecision::Allow);
    }

    #[test]
    fn agent_allowed_tool_passes() {
        let denied_tools = HashSet::from([String::from("dangerous.tool")]);
        let decision = evaluate(
            &PolicyContext {
                entrypoint: "cli".to_string(),
                tool_name: Some("safe.tool".to_string()),
                role: Role::Agent,
            },
            &denied_tools,
            true,
        );

        assert_eq!(decision, PolicyDecision::Allow);
    }

    #[test]
    fn default_deny_rejects_all_roles() {
        let denied_tools = HashSet::new();

        for role in [Role::Admin, Role::Agent] {
            let decision = evaluate(
                &PolicyContext {
                    entrypoint: "cli".to_string(),
                    tool_name: Some("safe.tool".to_string()),
                    role,
                },
                &denied_tools,
                false,
            );

            assert_eq!(
                decision,
                PolicyDecision::Deny {
                    reason: "default deny policy".to_string(),
                }
            );
        }
    }
}
