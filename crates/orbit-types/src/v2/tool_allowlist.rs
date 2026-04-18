use thiserror::Error;

/// Explicit list of permitted wildcard roots (§6 / §12 Q7).
///
/// Any wildcard like `orbit.graph.*` must have its root prefix on this list.
/// Top-level `orbit.*` is deliberately NOT permitted — the max-depth-2 rule
/// the design doc mentions is implemented here as an explicit allowlist so the
/// set grows deliberately and every reviewer sees the full scope.
pub const V2_TOOL_WILDCARD_ROOTS: &[&str] = &[
    "orbit.graph.",
    "orbit.task.",
    "orbit.audit.",
    "fs.",
    "proc.",
];

#[derive(Debug, Error)]
pub enum ToolAllowlistError {
    #[error("wildcard root not permitted: `{0}` (see V2_TOOL_WILDCARD_ROOTS)")]
    WildcardRootNotPermitted(String),
    #[error("empty tool name in allowlist")]
    EmptyName,
}

/// Validate an activity's declared tool allowlist at asset-load time.
/// Returns Ok(()) when every entry is a concrete tool name or a permitted
/// wildcard root. Does NOT verify that concrete tool names resolve in the
/// registry — that's a separate load-time check at the engine layer.
pub fn validate_tool_allowlist(allowlist: &[String]) -> Result<(), ToolAllowlistError> {
    for entry in allowlist {
        if entry.is_empty() {
            return Err(ToolAllowlistError::EmptyName);
        }
        if let Some(prefix) = wildcard_prefix(entry) {
            if !V2_TOOL_WILDCARD_ROOTS.iter().any(|root| *root == prefix) {
                return Err(ToolAllowlistError::WildcardRootNotPermitted(entry.clone()));
            }
        }
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
        if let Some(prefix) = wildcard_prefix(entry) {
            if tool_name.starts_with(&prefix) {
                return true;
            }
        }
    }
    false
}

fn wildcard_prefix(entry: &str) -> Option<String> {
    entry.strip_suffix('*').map(|s| s.to_string())
}
