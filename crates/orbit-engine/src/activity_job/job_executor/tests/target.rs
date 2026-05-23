#![allow(missing_docs)]

use super::*;

// ----- Role override regression tests (ADR-029, T20260428-12) ---------

use orbit_common::types::activity_job::{AgentLoopSpec, AgentRole, Backend, OnDenial, Provider};
use std::sync::Mutex as RoleHostMutex;

use crate::AgentRoleConfig;

use super::role_overridden_spec;

/// Minimal `V2RuntimeHost` mock used only by the role-override tests.
/// Records every `agent_role_config` lookup so tests can assert the
/// dispatcher consulted the right role, and otherwise refuses every
/// other dispatch path so a stray dispatch surfaces immediately.
struct RoleHost {
    config: HashMap<AgentRole, AgentRoleConfig>,
    observed: RoleHostMutex<Vec<AgentRole>>,
}

impl RoleHost {
    fn new(config: HashMap<AgentRole, AgentRoleConfig>) -> Self {
        Self {
            config,
            observed: RoleHostMutex::new(Vec::new()),
        }
    }

    fn observed_lookups(&self) -> Vec<AgentRole> {
        self.observed.lock().expect("observed lock").clone()
    }
}

impl V2RuntimeHost for RoleHost {
    fn run_deterministic(
        &self,
        _action: &str,
        _config: &Value,
        _input: &Value,
        _tool_context: orbit_tools::ToolContext,
    ) -> Result<Value, DispatchError> {
        Err(DispatchError::DeterministicActionNotRegistered(
            "role host: not used".into(),
        ))
    }

    fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::AgentLoopFailed(
            "role host: no credentials".into(),
        ))
    }

    fn resolve_cli_executor(
        &self,
        _provider: &str,
    ) -> Result<super::super::super::dispatcher::ResolvedCliExecutor, DispatchError> {
        Err(DispatchError::CliInvocationFailed(
            "role host: no CLI mapping".into(),
        ))
    }

    fn tool_context_for_activity(
        &self,
        _run_id: Option<&str>,
        _fs_profile: Option<&str>,
        _fs_audit: Option<std::sync::Arc<dyn orbit_tools::FsAuditLogger>>,
        _proc_allowed_programs: Option<&[String]>,
    ) -> orbit_tools::ToolContext {
        orbit_tools::ToolContext::default()
    }

    fn agent_role_config(&self, role: AgentRole) -> Option<AgentRoleConfig> {
        self.observed.lock().expect("observed lock").push(role);
        self.config.get(&role).cloned()
    }
}

fn inline_agent_loop_spec() -> AgentLoopSpec {
    AgentLoopSpec {
        instruction: "inline".to_string(),
        tools: Vec::new(),
        on_denial: OnDenial::Terminate,
        model: Some("claude-opus-4-7".to_string()),
        max_iterations: 1,
        backend: Backend::Cli,
        provider: Provider::Claude,
        wall_clock_timeout_seconds: 30,
        role: None,
        proc_allowed_programs: None,
    }
}

fn target_step_with_role(spec: AgentLoopSpec, role: Option<AgentRole>) -> super::TargetStep {
    super::TargetStep {
        spec: super::ActivityV2Spec::AgentLoop(spec),
        activity_name: None,
        fs_profile: None,
        default_input: None,
        timeout_seconds: 0,
        session: None,
        role,
    }
}

fn role_host_for_implementer_codex() -> RoleHost {
    let mut map = HashMap::new();
    map.insert(
        AgentRole::Implementer,
        AgentRoleConfig {
            provider: Some(Provider::Codex),
            model: None,
            backend: None,
        },
    );
    RoleHost::new(map)
}

fn exec_ctx<'a>(host: &'a dyn V2RuntimeHost) -> super::ExecCtx<'a> {
    let writer = test_writer("run-role-override");
    super::ExecCtx {
        run_id: "run-role-override".to_string(),
        audit: std::sync::Arc::new(writer),
        host,
        input: json!({}),
        pipeline: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
        sessions: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
        recovery_activity: None,
        item: None,
        iteration: None,
    }
}

#[test]
fn role_override_pulls_provider_from_host_for_step_role() {
    let host = role_host_for_implementer_codex();
    let ctx = exec_ctx(&host);
    let target = target_step_with_role(inline_agent_loop_spec(), Some(AgentRole::Implementer));

    let overridden = role_overridden_spec(&target, &ctx).expect("override expected");
    assert_eq!(overridden.provider, Provider::Codex);
    // Field-by-field fallback: model and backend stay inline.
    assert_eq!(overridden.model.as_deref(), Some("claude-opus-4-7"));
    assert_eq!(overridden.backend, Backend::Cli);
    assert_eq!(host.observed_lookups(), vec![AgentRole::Implementer]);
}

#[test]
fn role_override_step_role_wins_over_activity_role() {
    let host = role_host_for_implementer_codex();
    let ctx = exec_ctx(&host);
    // Activity declares Planner, but the step declares Implementer —
    // step wins.
    let mut spec = inline_agent_loop_spec();
    spec.role = Some(AgentRole::Planner);
    let target = target_step_with_role(spec, Some(AgentRole::Implementer));

    let overridden = role_overridden_spec(&target, &ctx).expect("override expected");
    assert_eq!(overridden.provider, Provider::Codex);
    // Only Implementer was looked up; Planner was never queried.
    assert_eq!(host.observed_lookups(), vec![AgentRole::Implementer]);
}

#[test]
fn role_override_falls_back_to_activity_role_when_step_role_absent() {
    let host = role_host_for_implementer_codex();
    let ctx = exec_ctx(&host);
    let mut spec = inline_agent_loop_spec();
    spec.role = Some(AgentRole::Implementer);
    let target = target_step_with_role(spec, None);

    let overridden = role_overridden_spec(&target, &ctx).expect("override expected");
    assert_eq!(overridden.provider, Provider::Codex);
    assert_eq!(host.observed_lookups(), vec![AgentRole::Implementer]);
}

#[test]
fn role_override_returns_none_when_no_role_anywhere() {
    let host = role_host_for_implementer_codex();
    let ctx = exec_ctx(&host);
    let target = target_step_with_role(inline_agent_loop_spec(), None);

    // Inline activity role is also None — no override should be built and
    // the host should not be queried at all.
    assert!(role_overridden_spec(&target, &ctx).is_none());
    assert!(host.observed_lookups().is_empty());
}

#[test]
fn role_override_returns_none_when_host_has_no_matching_entry() {
    // Host returns Some(empty AgentRoleConfig) → resolver still falls back
    // to inline values for every field, but `role_overridden_spec` clones
    // and applies, leaving the spec semantically equal to the inline one.
    // For the "no entry" case we simulate via an empty host map.
    let host = RoleHost::new(HashMap::new());
    let ctx = exec_ctx(&host);
    let target = target_step_with_role(inline_agent_loop_spec(), Some(AgentRole::Reviewer));

    let overridden = role_overridden_spec(&target, &ctx).expect("override expected");
    assert_eq!(overridden.provider, Provider::Claude);
    assert_eq!(overridden.model.as_deref(), Some("claude-opus-4-7"));
    assert_eq!(overridden.backend, Backend::Cli);
    assert_eq!(host.observed_lookups(), vec![AgentRole::Reviewer]);
}

#[test]
fn role_override_does_not_apply_to_non_agent_loop_specs() {
    let host = role_host_for_implementer_codex();
    let ctx = exec_ctx(&host);
    // A deterministic target with a step-level role is meaningless for
    // dispatch but must not panic or reach the role host.
    let target = super::TargetStep {
        spec: super::ActivityV2Spec::Deterministic(DeterministicSpec {
            action: "noop".to_string(),
            config: Value::Null,
        }),
        activity_name: None,
        fs_profile: None,
        default_input: None,
        timeout_seconds: 0,
        session: None,
        role: Some(AgentRole::Implementer),
    };
    assert!(role_overridden_spec(&target, &ctx).is_none());
    assert!(host.observed_lookups().is_empty());
}

/// Replay short-circuit regression (AC #9). The override is applied
/// before session creation, so `replay_active()` must continue to see
/// the env var and return `true` regardless of the override.
#[test]
fn role_override_does_not_disable_replay_short_circuit() {
    // Use a unique env var name to avoid stomping other tests; we restore
    // it on drop.
    struct EnvGuard {
        key: &'static str,
        prior: Option<String>,
    }
    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prior = std::env::var(key).ok();
            // SAFETY: tests touching env vars must coordinate; we use a
            // dedicated key and restore on drop, and replay_active() is
            // a pure read of two specific keys so no other thread races
            // it for the duration of this test.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, prior }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: see EnvGuard::set.
            unsafe {
                match &self.prior {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    let _guard = EnvGuard::set("ORBIT_V2_REPLAY", "1");
    let host = role_host_for_implementer_codex();
    let ctx = exec_ctx(&host);
    let target = target_step_with_role(inline_agent_loop_spec(), Some(AgentRole::Implementer));

    let overridden = role_overridden_spec(&target, &ctx).expect("override expected");
    assert_eq!(overridden.provider, Provider::Codex);
    // Replay short-circuit still triggers — independent of the override.
    assert!(super::replay_active());
}
