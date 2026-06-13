use std::collections::BTreeSet;

use thiserror::Error;

use super::activity_v2::{ActivityV2, ActivityV2Spec};

/// Explicit list of permitted wildcard roots (§6 / §12 Q7).
///
/// Any wildcard like `orbit.graph.*` must have its root prefix on this list.
/// Top-level `orbit.*` is deliberately NOT permitted — the max-depth-2 rule
/// the design doc mentions is implemented here as an explicit allowlist so the
/// set grows deliberately and every reviewer sees the full scope.
pub const V2_TOOL_WILDCARD_ROOTS: &[&str] = &[
    "orbit.graph.",
    "orbit.friction.",
    "orbit.learning.",
    "orbit.task.",
    "orbit.state.",
    "orbit.semantic.",
    // Reserved for audit-session tools. No builtin tools currently live under
    // this root, so registry validation treats it as intentionally empty.
    "orbit.audit.",
    "fs.",
    "proc.",
];

pub const V2_INTENTIONALLY_EMPTY_TOOL_WILDCARD_ROOTS: &[&str] = &["orbit.audit."];

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ToolAllowlistError {
    #[error(
        "wildcard root not permitted in allowlist entry `{entry}` (see V2_TOOL_WILDCARD_ROOTS)"
    )]
    WildcardRootNotPermitted { entry: String },
    #[error("empty tool name in allowlist entry at index {index}")]
    EmptyName { index: usize },
    #[error("unknown tool name in allowlist entry `{entry}`")]
    UnknownToolName { entry: String },
    #[error("wildcard allowlist entry `{entry}` did not match any registered tools")]
    WildcardRootMatchesNoTools { entry: String },
}

/// Validate an activity's declared tool allowlist at asset-load time.
/// Returns Ok(()) when every entry is a concrete tool name or a permitted
/// wildcard root. Does NOT verify that concrete tool names resolve in the
/// registry — that's a separate load-time check at the engine layer.
pub fn validate_tool_allowlist(allowlist: &[String]) -> Result<(), ToolAllowlistError> {
    for (index, entry) in allowlist.iter().enumerate() {
        if entry.trim().is_empty() {
            return Err(ToolAllowlistError::EmptyName { index });
        }
        if let Some(prefix) = wildcard_prefix(entry)
            && !V2_TOOL_WILDCARD_ROOTS.contains(&prefix)
        {
            return Err(ToolAllowlistError::WildcardRootNotPermitted {
                entry: entry.clone(),
            });
        }
    }
    Ok(())
}

/// Validate an activity allowlist against the registered tool surface.
///
/// Concrete names must resolve exactly. Wildcards must use an approved root,
/// and must expand to at least one registered tool unless the root is an
/// explicitly documented empty reservation.
pub fn validate_tool_allowlist_against_registered_tools<'a, I>(
    allowlist: &[String],
    registered_tools: I,
) -> Result<(), ToolAllowlistError>
where
    I: IntoIterator<Item = &'a str>,
{
    validate_tool_allowlist(allowlist)?;

    let registered_tools: BTreeSet<&str> = registered_tools.into_iter().collect();
    for entry in allowlist {
        if let Some(prefix) = wildcard_prefix(entry) {
            let has_match = registered_tools.iter().any(|tool| tool.starts_with(prefix));
            if !has_match && !V2_INTENTIONALLY_EMPTY_TOOL_WILDCARD_ROOTS.contains(&prefix) {
                return Err(ToolAllowlistError::WildcardRootMatchesNoTools {
                    entry: entry.clone(),
                });
            }
            continue;
        }

        if !registered_tools.contains(entry.as_str()) {
            return Err(ToolAllowlistError::UnknownToolName {
                entry: entry.clone(),
            });
        }
    }

    Ok(())
}

pub fn validate_activity_tool_allowlist(activity: &ActivityV2) -> Result<(), ToolAllowlistError> {
    if let Some(allowlist) = activity_tool_allowlist(activity) {
        validate_tool_allowlist(allowlist)?;
    }
    Ok(())
}

pub fn validate_activity_tool_allowlist_against_registered_tools<'a, I>(
    activity: &ActivityV2,
    registered_tools: I,
) -> Result<(), ToolAllowlistError>
where
    I: IntoIterator<Item = &'a str>,
{
    if let Some(allowlist) = activity_tool_allowlist(activity) {
        validate_tool_allowlist_against_registered_tools(allowlist, registered_tools)?;
    }
    Ok(())
}

/// Runtime check: is `tool_name` permitted by `allowlist`?
/// Empty allowlist means nothing is allowed (explicit policy per §6.1).
pub fn tool_allowed(tool_name: &str, allowlist: &[String]) -> bool {
    for entry in allowlist {
        if entry == tool_name {
            return true;
        }
        if let Some(prefix) = wildcard_prefix(entry)
            && tool_name.starts_with(prefix)
        {
            return true;
        }
    }
    false
}

fn activity_tool_allowlist(activity: &ActivityV2) -> Option<&[String]> {
    match &activity.spec {
        ActivityV2Spec::AgentLoop(spec) => Some(&spec.tools),
        ActivityV2Spec::Groundhog(spec) => Some(&spec.tools),
        ActivityV2Spec::Deterministic(_) => None,
    }
}

fn wildcard_prefix(entry: &str) -> Option<&str> {
    entry.strip_suffix('*')
}
