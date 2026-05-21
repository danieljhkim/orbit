//! Authoritative agent-family helpers and named crew resolution.
//!
//! This is the single source of truth Orbit consults whenever an activity needs
//! to embed a model duo into its instructions. Splitting the heavy "judgment"
//! model from a cheaper "implementation" helper makes execution mode
//! deterministic per agent family rather than depending on per-prompt edits.
//!
//! Activities reference the resolved pair via the `{{orchestrator_model}}`,
//! `{{helper_model}}`, and `{{agent_family}}` placeholders, which the runtime
//! substitutes into the instruction text before invoking the agent.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{AgentFamily, OrbitError};

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

/// One role assignment inside a named crew.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrewRoleAssignment {
    pub model: String,
    pub provider: String,
    pub backend: String,
}

/// A named planner/implementer/reviewer lineup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crew {
    pub name: String,
    pub planner: CrewRoleAssignment,
    pub implementer: CrewRoleAssignment,
    pub reviewer: CrewRoleAssignment,
}

impl Crew {
    pub fn role(&self, role: &str) -> Option<&CrewRoleAssignment> {
        match role {
            "planner" => Some(&self.planner),
            "implementer" => Some(&self.implementer),
            "reviewer" => Some(&self.reviewer),
            _ => None,
        }
    }
}

/// Resolve a named crew from the active registry.
pub fn resolve_crew(name: &str, registry: &BTreeMap<String, Crew>) -> Result<Crew, OrbitError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::invalid_input_with_suggestions(
            "crew name must not be empty",
            registry.keys().cloned().collect(),
        ));
    }
    registry.get(trimmed).cloned().ok_or_else(|| {
        OrbitError::invalid_input_with_suggestions(
            format!("crew '{trimmed}' is not defined in [crews.*]"),
            registry.keys().cloned().collect(),
        )
    })
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
    [
        AgentFamily::Codex.as_str(),
        AgentFamily::Claude.as_str(),
        AgentFamily::Gemini.as_str(),
        AgentFamily::Grok.as_str(),
    ]
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

#[cfg(test)]
mod tests;
