#![allow(missing_docs)]

use orbit_common::test_fixtures::{TEST_CLAUDE_MODEL, TEST_CODEX_MODEL};
use orbit_types::workflow::activity_job::{AgentLoopSpec, OnDenial, Provider};

use crate::context::CrewConfig;

use super::super::crew::{ResolvedAgentSettings, apply_resolved_settings, resolve_from_config};

fn inline_spec() -> AgentLoopSpec {
    AgentLoopSpec {
        instruction: String::new(),
        tools: Vec::new(),
        on_denial: OnDenial::Terminate,
        model: Some(TEST_CLAUDE_MODEL.to_string()),
        max_iterations: 1,
        backend: None,
        provider: Provider::Claude,
        wall_clock_timeout_seconds: 30,
        require_response_envelope: false,
        require_completion_envelope: true,
        proc_allowed_programs: None,
    }
}

#[test]
fn partial_crew_assignment_keeps_inline_fields() {
    let config = CrewConfig {
        provider: Some(Provider::Codex),
        model: None,
    };
    let resolved = resolve_from_config(&config, &inline_spec());
    assert_eq!(resolved.provider, Provider::Codex);
    assert_eq!(resolved.model.as_deref(), Some(TEST_CLAUDE_MODEL));
}

#[test]
fn full_crew_assignment_replaces_every_field() {
    let config = CrewConfig {
        provider: Some(Provider::Codex),
        model: Some(TEST_CODEX_MODEL.to_string()),
    };
    let resolved = resolve_from_config(&config, &inline_spec());
    assert_eq!(resolved.provider, Provider::Codex);
    assert_eq!(resolved.model.as_deref(), Some(TEST_CODEX_MODEL));
}

#[test]
fn apply_mutates_spec_in_place() {
    let mut spec = inline_spec();
    let resolved = ResolvedAgentSettings {
        provider: Provider::Codex,
        model: Some(TEST_CODEX_MODEL.to_string()),
    };
    apply_resolved_settings(&mut spec, &resolved);
    assert_eq!(spec.provider, Provider::Codex);
    assert_eq!(spec.model.as_deref(), Some(TEST_CODEX_MODEL));
}
