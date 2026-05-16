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

use super::OrbitError;

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
/// made assumptions about the previous number of families.
pub const fn all_agent_families() -> [&'static str; 4] {
    ["codex", "claude", "gemini", "grok"]
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

/// Best-effort reverse mapping from an exact model string to the agent CLI
/// family that would invoke it.
///
/// Orbit stores model-only attribution on tasks, but some execution paths still
/// need to recover the agent family for provider dispatch. This helper accepts
/// both the new exact model strings (for example `claude-opus-4.6`) and the
/// older shorthand values that may still appear in legacy artifacts.
pub fn infer_agent_family_from_model(model: &str) -> Option<String> {
    let model = model.trim().to_ascii_lowercase();
    if model.is_empty() {
        return None;
    }

    if model.starts_with("gpt-") || model.starts_with("o1") || model.starts_with("o3") {
        return Some("codex".to_string());
    }
    if model.starts_with("claude-") || model.starts_with("opus") || model.starts_with("sonnet") {
        return Some("claude".to_string());
    }
    if model.starts_with("gemini-") {
        return Some("gemini".to_string());
    }
    // Grok (xAI) — supports both grok-4 style and the shorter grok3* naming
    if model.starts_with("grok-") || model.starts_with("grok3") {
        return Some("grok".to_string());
    }

    None
}

/// Normalize an optional legacy agent family and optional model into the agent
/// family implied by the pair.
///
/// `model` is the preferred provenance field for tool calls. When it names a
/// known Orbit provider family, this helper infers the agent family from the
/// model. Legacy callers may still pass `agent`; if both are present and the
/// model maps to a different family, Orbit rejects the inconsistent identity
/// instead of recording contradictory attribution.
pub fn normalize_agent_family_for_model(
    agent_cli: Option<&str>,
    model: Option<&str>,
) -> Result<Option<String>, OrbitError> {
    let agent = agent_cli
        .map(agent_family_from_cli)
        .filter(|value| !value.trim().is_empty());
    let model = model.map(str::trim).filter(|value| !value.is_empty());
    let inferred = model.and_then(infer_agent_family_from_model);

    if let (Some(agent), Some(inferred)) = (agent.as_deref(), inferred.as_deref())
        && agent != inferred
    {
        return Err(OrbitError::InvalidInput(format!(
            "`agent` '{agent}' does not match `model` '{}' (inferred agent family '{inferred}')",
            model.unwrap_or_default()
        )));
    }

    Ok(agent.or(inferred))
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
/// runtime configuration. `orbit-common::types` remains the fallback source of
/// truth for default model pairs.
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
            "pro",
            "flash",
        )),
        "grok" => Some(AgentModelPair::new("grok-4", "grok-3")),
        _ => None,
    }
}
