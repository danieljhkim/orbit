#![allow(missing_docs)]

use orbit_common::test_fixtures::{TEST_CLAUDE_MODEL, TEST_CODEX_MODEL};
use orbit_common::types::activity_job::{AgentLoopSpec, AgentRole, Backend, OnDenial, Provider};

use crate::context::AgentRoleConfig;

use super::super::agent_role::{
    ResolvedAgentSettings, apply_resolved_settings, resolve_from_config,
};

fn inline_spec() -> AgentLoopSpec {
    AgentLoopSpec {
        instruction: String::new(),
        tools: Vec::new(),
        on_denial: OnDenial::Terminate,
        model: Some(TEST_CLAUDE_MODEL.to_string()),
        max_iterations: 1,
        backend: Backend::Cli,
        provider: Provider::Claude,
        wall_clock_timeout_seconds: 30,
        role: Some(AgentRole::Implementer),
        proc_allowed_programs: None,
    }
}

#[test]
fn missing_config_yields_inline_values_unchanged() {
    let inline = inline_spec();
    let resolved = resolve_from_config(None, &inline);
    assert_eq!(resolved.provider, Provider::Claude);
    assert_eq!(resolved.model.as_deref(), Some(TEST_CLAUDE_MODEL));
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
    assert_eq!(resolved.model.as_deref(), Some(TEST_CLAUDE_MODEL));
    assert_eq!(resolved.backend, Backend::Cli);
}

#[test]
fn full_override_replaces_every_field() {
    let cfg = AgentRoleConfig {
        provider: Some(Provider::Codex),
        model: Some(TEST_CODEX_MODEL.to_string()),
        backend: Some(Backend::Http),
    };
    let inline = inline_spec();
    let resolved = resolve_from_config(Some(&cfg), &inline);
    assert_eq!(resolved.provider, Provider::Codex);
    assert_eq!(resolved.model.as_deref(), Some(TEST_CODEX_MODEL));
    assert_eq!(resolved.backend, Backend::Http);
}

#[test]
fn apply_mutates_spec_in_place() {
    let mut spec = inline_spec();
    let resolved = ResolvedAgentSettings {
        provider: Provider::Codex,
        model: Some(TEST_CODEX_MODEL.to_string()),
        backend: Backend::Http,
    };
    apply_resolved_settings(&mut spec, &resolved);
    assert_eq!(spec.provider, Provider::Codex);
    assert_eq!(spec.model.as_deref(), Some(TEST_CODEX_MODEL));
    assert_eq!(spec.backend, Backend::Http);
}
