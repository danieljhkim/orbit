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

/// Table-driven coverage of **step 2** of provider resolution (ORB-10091): the
/// selected crew role's `(provider, model, backend)` values override the
/// activity's inline `agent_loop` baseline, field by field. This is *not* the
/// crew-*selection* precedence (explicit > task_config > workspace_default >
/// environment_default > system_default) — that runs earlier, in orbit-core's
/// `select_crew_name` / `resolve_crew_for_task`, and is table-tested there.
///
/// Each row states the crew-config override and the expected resolved triple
/// against a fixed `Provider::Claude` inline spec. The rows where
/// `config.provider` is `None` assert the inline provider identity is preserved
/// and never re-defaulted to `Provider::default()`.
#[test]
fn resolve_from_config_precedence_table() {
    struct Row {
        name: &'static str,
        config: Option<AgentRoleConfig>,
        expect_provider: Provider,
        expect_model: Option<&'static str>,
        expect_backend: Backend,
    }

    let rows = [
        Row {
            name: "no crew config -> inline preserved",
            config: None,
            expect_provider: Provider::Claude,
            expect_model: Some(TEST_CLAUDE_MODEL),
            expect_backend: Backend::Cli,
        },
        Row {
            name: "provider-only override keeps inline model + backend",
            config: Some(AgentRoleConfig {
                provider: Some(Provider::Codex),
                model: None,
                backend: None,
            }),
            expect_provider: Provider::Codex,
            expect_model: Some(TEST_CLAUDE_MODEL),
            expect_backend: Backend::Cli,
        },
        Row {
            name: "full override replaces every field",
            config: Some(AgentRoleConfig {
                provider: Some(Provider::Codex),
                model: Some(TEST_CODEX_MODEL.to_string()),
                backend: Some(Backend::Http),
            }),
            expect_provider: Provider::Codex,
            expect_model: Some(TEST_CODEX_MODEL),
            expect_backend: Backend::Http,
        },
        Row {
            name: "model-only override preserves persisted provider",
            config: Some(AgentRoleConfig {
                provider: None,
                model: Some(TEST_CODEX_MODEL.to_string()),
                backend: None,
            }),
            expect_provider: Provider::Claude,
            expect_model: Some(TEST_CODEX_MODEL),
            expect_backend: Backend::Cli,
        },
        Row {
            name: "backend-only override preserves persisted provider + model",
            config: Some(AgentRoleConfig {
                provider: None,
                model: None,
                backend: Some(Backend::Http),
            }),
            expect_provider: Provider::Claude,
            expect_model: Some(TEST_CLAUDE_MODEL),
            expect_backend: Backend::Http,
        },
    ];

    let inline = inline_spec();
    for row in rows {
        let resolved = resolve_from_config(row.config.as_ref(), &inline);
        assert_eq!(
            resolved.provider, row.expect_provider,
            "provider: {}",
            row.name
        );
        assert_eq!(
            resolved.model.as_deref(),
            row.expect_model,
            "model: {}",
            row.name
        );
        assert_eq!(
            resolved.backend, row.expect_backend,
            "backend: {}",
            row.name
        );
    }
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
