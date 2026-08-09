//! Per-role agent settings resolver (ADR-029).
//!
//! Bridges the role tag on an `agent_loop` activity (or its
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
use serde_json::Value;

use crate::context::AgentRoleConfig;

use super::dispatcher::{DispatchError, V2RuntimeHost};

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

/// Resolve the flat crew selected explicitly by a rendered activity input.
/// Returns `None` only when the input did not select a crew, leaving untagged
/// activities on their inline baseline. A selected crew that the host cannot
/// materialize is an error rather than an inline-provider fallback.
pub fn resolve_explicit_crew_settings(
    host: &dyn V2RuntimeHost,
    inline: &AgentLoopSpec,
    input: &serde_json::Value,
) -> Result<Option<ResolvedAgentSettings>, DispatchError> {
    let Some(config) = host.explicit_agent_crew_config_for_input(input)? else {
        return Ok(None);
    };
    Ok(Some(resolve_from_config(Some(&config), inline)))
}

/// Add the configured system crew to an activity input that explicitly opts
/// into the system route. The marker is asset data, while the crew is read
/// from the runtime host for each dispatch.
pub fn inject_system_crew_input(
    host: &dyn V2RuntimeHost,
    input: &Value,
) -> Result<Value, DispatchError> {
    if input.get("system_crew").and_then(Value::as_bool) != Some(true) {
        return Ok(input.clone());
    }
    let crew = host.system_crew_for_dispatch().ok_or_else(|| {
        DispatchError::JobValidation(
            "system activity requests `workflow.system_crew`, but this runtime host does not provide that configuration"
                .to_string(),
        )
    })?;
    let mut input = input.clone();
    let object = input.as_object_mut().ok_or_else(|| {
        DispatchError::JobValidation(
            "system activity requests `workflow.system_crew`, but its input must be an object"
                .to_string(),
        )
    })?;
    object.insert("crew".to_string(), Value::String(crew));
    object.insert(
        "crew_config_key".to_string(),
        Value::String("workflow.system_crew".to_string()),
    );
    Ok(input)
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
