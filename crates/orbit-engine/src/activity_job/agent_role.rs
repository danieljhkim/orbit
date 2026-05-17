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
mod tests {
    use super::*;
    use orbit_common::types::activity_job::OnDenial;

    fn inline_spec() -> AgentLoopSpec {
        AgentLoopSpec {
            instruction: String::new(),
            tools: Vec::new(),
            on_denial: OnDenial::Terminate,
            model: Some("claude-opus-4-7".to_string()),
            max_iterations: 1,
            backend: Backend::Cli,
            provider: Provider::Claude,
            wall_clock_timeout_seconds: 30,
            role: Some(AgentRole::Implementer),
        }
    }

    #[test]
    fn missing_config_yields_inline_values_unchanged() {
        let inline = inline_spec();
        let resolved = resolve_from_config(None, &inline);
        assert_eq!(resolved.provider, Provider::Claude);
        assert_eq!(resolved.model.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(resolved.backend, Backend::Cli);
    }

    #[test]
    fn provider_only_override_keeps_inline_model_and_backend() {
        let cfg = AgentRoleConfig {
            provider: Some(Provider::Codex),
            model: None,
            backend: None,
        };
        let inline = inline_spec();
        let resolved = resolve_from_config(Some(&cfg), &inline);
        assert_eq!(resolved.provider, Provider::Codex);
        assert_eq!(resolved.model.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(resolved.backend, Backend::Cli);
    }

    #[test]
    fn full_override_replaces_every_field() {
        let cfg = AgentRoleConfig {
            provider: Some(Provider::Codex),
            model: Some("gpt-5.5".to_string()),
            backend: Some(Backend::Http),
        };
        let inline = inline_spec();
        let resolved = resolve_from_config(Some(&cfg), &inline);
        assert_eq!(resolved.provider, Provider::Codex);
        assert_eq!(resolved.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(resolved.backend, Backend::Http);
    }

    #[test]
    fn apply_mutates_spec_in_place() {
        let mut spec = inline_spec();
        let resolved = ResolvedAgentSettings {
            provider: Provider::Codex,
            model: Some("gpt-5.5".to_string()),
            backend: Backend::Http,
        };
        apply_resolved_settings(&mut spec, &resolved);
        assert_eq!(spec.provider, Provider::Codex);
        assert_eq!(spec.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(spec.backend, Backend::Http);
    }
}
