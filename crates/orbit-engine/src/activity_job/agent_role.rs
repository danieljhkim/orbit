//! Per-role agent settings resolver (ADR-029).
//!
//! Bridges the role tag on an `agent_loop` / `groundhog` activity (or its
//! enclosing `TargetStep`) to the selected `[crews.<name>]` role assignment.
//! The host returns parsed [`AgentRoleConfig`] values, and this module
//! collapses them with the inline `provider`, `model`, and `backend` fields
//! on the activity into a single [`ResolvedAgentSettings`] triple.
//!
//! # Precedence
//!
//! For each field independently:
//! 1. The matching field from the selected crew if the host returned `Some`.
//! 2. Otherwise the inline value on the activity's [`AgentLoopSpec`].
//!
//! No validation happens here — `Provider`/`Backend` were already parsed at
//! the orbit-core boundary. Unknown strings yield `None` for that field, so a
//! typo'd config does not silently coerce dispatch onto a wrong runtime.

use orbit_common::types::activity_job::{AgentLoopSpec, AgentRole, Backend, Provider};

use crate::context::AgentRoleConfig;

use super::dispatcher::V2RuntimeHost;

/// Resolved `(provider, model, backend)` triple ready to apply to a cloned
/// [`AgentLoopSpec`] before downstream dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgentSettings {
    pub provider: Provider,
    pub model: Option<String>,
    pub backend: Backend,
}

/// Resolve role-specific overrides from the host with field-by-field fallback
/// to the inline activity values. Pure function — no I/O beyond the host
/// callback.
pub fn resolve_agent_settings(
    role: AgentRole,
    host: &dyn V2RuntimeHost,
    inline: &AgentLoopSpec,
    input: &serde_json::Value,
) -> ResolvedAgentSettings {
    let config = host.agent_role_config_for_input(role, input);
    resolve_from_config(config.as_ref(), inline)
}

/// Pure helper used by both the host-driven path and the unit tests so the
/// fallback rules stay in one place.
pub(crate) fn resolve_from_config(
    config: Option<&AgentRoleConfig>,
    inline: &AgentLoopSpec,
) -> ResolvedAgentSettings {
    ResolvedAgentSettings {
        provider: config.and_then(|c| c.provider).unwrap_or(inline.provider),
        model: config
            .and_then(|c| c.model.clone())
            .or_else(|| inline.model.clone()),
        backend: config.and_then(|c| c.backend).unwrap_or(inline.backend),
    }
}

/// Apply a [`ResolvedAgentSettings`] triple onto an existing [`AgentLoopSpec`]
/// in place. Used by the dispatcher to mutate the cloned spec before invoking
/// the runner.
pub fn apply_resolved_settings(spec: &mut AgentLoopSpec, resolved: &ResolvedAgentSettings) {
    spec.provider = resolved.provider;
    spec.model = resolved.model.clone();
    spec.backend = resolved.backend;
}

#[cfg(test)]
mod tests;
