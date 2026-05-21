use orbit_common::types::activity_job::{AgentLoopSpec, AgentRole, Backend, OnDenial, Provider};

use crate::context::AgentRoleConfig;

use super::super::{ResolvedAgentSettings, apply_resolved_settings, resolve_from_config};

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
