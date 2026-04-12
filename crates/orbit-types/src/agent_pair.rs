//! Authoritative mapping from an agent CLI family to the orchestrator/helper
//! model pair that should drive bounded implementation work.
//!
//! This is the single source of truth Orbit consults whenever an activity needs
//! to embed a model duo into its instructions. Splitting the heavy "judgment"
//! model from a cheaper "implementation" helper makes execution mode
//! deterministic per agent family rather than depending on per-prompt edits.
//!
//! Activities reference the resolved pair via the `{{orchestrator_model}}`,
//! `{{helper_model}}`, and `{{agent_family}}` placeholders, which the runtime
//! substitutes into the instruction text before invoking the agent.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// A resolved (orchestrator, helper) duo for a given agent family.
///
/// - `orchestrator` owns plan, review, and integration responsibilities.
/// - `helper` owns the bounded implementation work delegated by the orchestrator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentModelPair {
    pub orchestrator: String,
    pub helper: String,
}

impl AgentModelPair {
    pub fn new(orchestrator: impl Into<String>, helper: impl Into<String>) -> Self {
        Self {
            orchestrator: orchestrator.into(),
            helper: helper.into(),
        }
    }
}

/// The full set of agent CLI families Orbit knows how to orchestrate.
///
/// This is the single source of truth for the candidate set used by
/// cross-agent workflows (e.g. the `duel` evaluation harness), so adding
/// a new family here automatically includes it in future permutations
/// without touching any other module.
///
/// The return type is a fixed-size array rather than a `Vec` so the
/// cardinality is enforced at compile time: adding a family requires
/// changing the array size, which in turn surfaces any call site that
/// made assumptions about exactly three families.
pub const fn all_agent_families() -> [&'static str; 3] {
    ["codex", "claude", "gemini"]
}

/// Normalize an `agent_cli` value into a stable, lowercased family identifier
/// (e.g. `/usr/local/bin/Codex` -> `codex`).
pub fn agent_family_from_cli(agent_cli: &str) -> String {
    Path::new(agent_cli)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(agent_cli)
        .to_ascii_lowercase()
}

/// Resolve the orchestrator/helper model pair for an `agent_cli`.
///
/// Returns `None` for unknown agent families. Callers that need a fallback
/// (for example, the runtime envelope renderer) decide what placeholder text
/// to inject when no mapping is registered.
pub fn resolve_agent_model_pair(agent_cli: &str) -> Option<AgentModelPair> {
    resolve_agent_model_pair_or(agent_cli, None)
}

/// Resolve the orchestrator/helper model pair for an `agent_cli`, allowing a
/// caller-supplied override to replace the built-in defaults.
///
/// This is the config-aware hook used by upstream crates that have access to
/// runtime configuration. `orbit-types` remains the fallback source of truth
/// for default model pairs.
pub fn resolve_agent_model_pair_or(
    agent_cli: &str,
    config_override: Option<&AgentModelPair>,
) -> Option<AgentModelPair> {
    if let Some(override_pair) = config_override {
        return Some(override_pair.clone());
    }

    let family = agent_family_from_cli(agent_cli);
    match family.as_str() {
        "codex" => Some(AgentModelPair::new("gpt-5.4", "gpt-5.4-mini")),
        "claude" => Some(AgentModelPair::new("opus-4.6", "sonnet-4.6")),
        "gemini" => Some(AgentModelPair::new(
            "gemini-3.1-pro-preview",
            "gemini-3-flash-preview",
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_normalizes_path_and_case() {
        assert_eq!(agent_family_from_cli("codex"), "codex");
        assert_eq!(agent_family_from_cli("/usr/local/bin/Codex"), "codex");
        assert_eq!(agent_family_from_cli("/opt/CLAUDE"), "claude");
    }

    #[test]
    fn resolves_codex_pair() {
        let pair = resolve_agent_model_pair("codex").expect("codex pair");
        assert_eq!(pair.orchestrator, "gpt-5.4");
        assert_eq!(pair.helper, "gpt-5.4-mini");
    }

    #[test]
    fn resolves_claude_pair() {
        let pair = resolve_agent_model_pair("claude").expect("claude pair");
        assert_eq!(pair.orchestrator, "opus-4.6");
        assert_eq!(pair.helper, "sonnet-4.6");
    }

    #[test]
    fn resolves_gemini_pair() {
        let pair = resolve_agent_model_pair("gemini").expect("gemini pair");
        assert_eq!(pair.orchestrator, "gemini-3.1-pro-preview");
        assert_eq!(pair.helper, "gemini-3-flash-preview");
    }

    #[test]
    fn resolves_path_prefixed_cli() {
        let pair =
            resolve_agent_model_pair("/usr/local/bin/codex").expect("path-prefixed codex pair");
        assert_eq!(pair.orchestrator, "gpt-5.4");
        assert_eq!(pair.helper, "gpt-5.4-mini");
    }

    #[test]
    fn override_pair_takes_precedence_over_defaults() {
        let override_pair = AgentModelPair::new("opus-4.7", "sonnet-4.7");
        let pair =
            resolve_agent_model_pair_or("claude", Some(&override_pair)).expect("override pair");
        assert_eq!(pair, override_pair);
    }

    #[test]
    fn unknown_agent_returns_none() {
        assert!(resolve_agent_model_pair("mock-agent").is_none());
        assert!(resolve_agent_model_pair("").is_none());
    }

    #[test]
    fn all_agent_families_lists_codex_claude_gemini_in_order() {
        assert_eq!(all_agent_families(), ["codex", "claude", "gemini"]);
    }

    #[test]
    fn all_agent_families_members_resolve_to_real_pairs() {
        for family in all_agent_families() {
            assert!(
                resolve_agent_model_pair(family).is_some(),
                "family {family:?} has no registered model pair"
            );
        }
    }
}
