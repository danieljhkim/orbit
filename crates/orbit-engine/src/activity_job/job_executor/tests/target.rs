#![allow(missing_docs)]

use super::*;

// ----- Role override regression tests (ADR-029, T20260428-12) ---------

use orbit_common::test_fixtures::TEST_CLAUDE_MODEL;
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

    fn explicit_agent_crew_config_for_input(
        &self,
        input: &Value,
    ) -> Result<Option<AgentRoleConfig>, DispatchError> {
        if input.get("crew").and_then(Value::as_str).is_none() {
            return Ok(None);
        }
        self.config
            .get(&AgentRole::Reviewer)
            .cloned()
            .map(Some)
            .ok_or_else(|| {
                DispatchError::JobValidation("explicit test crew is unknown".to_string())
            })
    }
}

fn inline_agent_loop_spec() -> AgentLoopSpec {
    AgentLoopSpec {
        instruction: "inline".to_string(),
        tools: Vec::new(),
        on_denial: OnDenial::Terminate,
        model: Some(TEST_CLAUDE_MODEL.to_string()),
        max_iterations: 1,
        backend: Backend::Cli,
        provider: Provider::Claude,
        wall_clock_timeout_seconds: 30,
        require_response_envelope: false,
        require_completion_envelope: true,
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
        failure_activity: None,
        item: None,
        iteration: None,
    }
}

#[test]
fn role_override_pulls_provider_from_host_for_step_role() {
    let host = role_host_for_implementer_codex();
    let ctx = exec_ctx(&host);
    let target = target_step_with_role(inline_agent_loop_spec(), Some(AgentRole::Implementer));

    let overridden = role_overridden_spec(&target, &ctx, &ctx.input)
        .expect("resolve override")
        .expect("override expected");
    assert_eq!(overridden.provider, Provider::Codex);
    // Field-by-field fallback: model and backend stay inline.
    assert_eq!(overridden.model.as_deref(), Some(TEST_CLAUDE_MODEL));
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

    let overridden = role_overridden_spec(&target, &ctx, &ctx.input)
        .expect("resolve override")
        .expect("override expected");
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

    let overridden = role_overridden_spec(&target, &ctx, &ctx.input)
        .expect("resolve override")
        .expect("override expected");
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
    assert!(
        role_overridden_spec(&target, &ctx, &ctx.input)
            .expect("resolve override")
            .is_none()
    );
    assert!(host.observed_lookups().is_empty());
}

#[test]
fn explicit_flat_crew_overrides_untagged_agent_activity() {
    let mut config = HashMap::new();
    config.insert(
        AgentRole::Reviewer,
        AgentRoleConfig {
            provider: Some(Provider::Codex),
            model: Some("gpt-review".to_string()),
            backend: Some(Backend::Cli),
        },
    );
    let host = RoleHost::new(config);
    let ctx = exec_ctx(&host);
    let rendered_input = json!({ "crew": "assessor" });
    let target = target_step_with_role(inline_agent_loop_spec(), None);

    let overridden = role_overridden_spec(&target, &ctx, &rendered_input)
        .expect("resolve explicit rendered crew")
        .expect("explicit rendered crew override");
    assert_eq!(overridden.provider, Provider::Codex);
    assert_eq!(overridden.model.as_deref(), Some("gpt-review"));
    assert_eq!(overridden.backend, Backend::Cli);
    assert!(host.observed_lookups().is_empty());
}

#[test]
fn explicit_flat_crew_resolution_failure_does_not_fall_back_inline() {
    let host = RoleHost::new(HashMap::new());
    let ctx = exec_ctx(&host);
    let rendered_input = json!({ "crew": "unknown-review" });
    let target = target_step_with_role(inline_agent_loop_spec(), None);

    let error = role_overridden_spec(&target, &ctx, &rendered_input)
        .expect_err("unknown explicit crew must fail closed");
    assert!(matches!(error, DispatchError::JobValidation(_)));
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

    let overridden = role_overridden_spec(&target, &ctx, &ctx.input)
        .expect("resolve override")
        .expect("override expected");
    assert_eq!(overridden.provider, Provider::Claude);
    assert_eq!(overridden.model.as_deref(), Some(TEST_CLAUDE_MODEL));
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
    assert!(
        role_overridden_spec(&target, &ctx, &ctx.input)
            .expect("resolve override")
            .is_none()
    );
    assert!(host.observed_lookups().is_empty());
}

/// Replay short-circuit regression (AC #9). Role resolution must not alter
/// the shared feature-gated replay predicate.
#[test]
fn role_override_does_not_change_feature_gated_replay_state() {
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
            // dedicated key and restore on drop.
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

    let overridden = role_overridden_spec(&target, &ctx, &ctx.input)
        .expect("resolve override")
        .expect("override expected");
    assert_eq!(overridden.provider, Provider::Codex);
    assert_eq!(
        crate::activity_job::agent_loop_driver::replay_active(),
        cfg!(feature = "replay")
    );
}

// ----- Telemetry persistence is non-fatal (ORB-10367) -----------------

/// Host whose invocation-trace persistence always fails, standing in for the
/// production failure mode where the store's `invocations` table lacks a
/// column the insert binds.
struct FailingTelemetryHost;

impl V2RuntimeHost for FailingTelemetryHost {
    fn run_deterministic(
        &self,
        _action: &str,
        _config: &Value,
        _input: &Value,
        _tool_context: orbit_tools::ToolContext,
    ) -> Result<Value, DispatchError> {
        Err(DispatchError::DeterministicActionNotRegistered(
            "failing telemetry host: not used".into(),
        ))
    }

    fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::AgentLoopFailed(
            "failing telemetry host: no credentials".into(),
        ))
    }

    fn resolve_cli_executor(
        &self,
        _provider: &str,
    ) -> Result<super::super::super::dispatcher::ResolvedCliExecutor, DispatchError> {
        Err(DispatchError::CliInvocationFailed(
            "failing telemetry host: no CLI mapping".into(),
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

    fn persist_invocation_trace(
        &self,
        _job_run_id: &str,
        _activity_id: &str,
        _provider: &str,
        _model: Option<&str>,
        _input: &Value,
        _trace: &orbit_common::types::InvocationTrace,
    ) -> Result<(), DispatchError> {
        Err(DispatchError::JobExecution(
            "persist invocation trace: store error: table invocations has no column named \
             cache_create_1h_tokens"
                .to_string(),
        ))
    }
}

fn dispatch_with_invocation() -> super::super::super::dispatcher::DispatchOutcome {
    super::super::super::dispatcher::DispatchOutcome {
        success: true,
        output: json!({ "implemented": true }),
        message: None,
        invocation: Some(super::super::super::dispatcher::DispatchInvocationTrace {
            provider: "claude".to_string(),
            model: Some(TEST_CLAUDE_MODEL.to_string()),
            trace: orbit_common::types::InvocationTrace::default(),
        }),
    }
}

/// [ORB-10367] A failed telemetry write must not discard completed agent
/// work. `persist_dispatch_invocation` returns `()` — there is no error path
/// left to propagate into the step outcome — and records the failure instead.
#[test]
fn failed_invocation_trace_persist_does_not_fail_the_step() {
    let host = FailingTelemetryHost;
    let ctx = exec_ctx(&host);
    let dispatch = dispatch_with_invocation();

    let trace = capture(|| {
        super::persist_dispatch_invocation(&ctx, "implement_one", &ctx.input, &dispatch);
    });

    // Logged loudly: an ERROR naming the run, the step, and the store error.
    let logged = trace
        .events
        .iter()
        .find(|event| event.target == "orbit.engine.telemetry")
        .expect("telemetry failure logged");
    assert_eq!(logged.level, Level::ERROR);
    assert_eq!(logged.field("step_id"), Some("implement_one"));
    assert!(
        logged
            .field("error")
            .expect("error field")
            .contains("cache_create_1h_tokens"),
        "unexpected error field: {:?}",
        logged.field("error")
    );

    // ...and surfaced on the run record as a telemetry.persist_failed event.
    assert_eq!(ctx.audit.telemetry_failure_count(), 1);
    assert!(ctx.audit.degraded_telemetry());
    let events = ctx.audit.events_snapshot().expect("events snapshot");
    let recorded = events
        .iter()
        .find(|event| event.envelope.event_type == "telemetry.persist_failed")
        .expect("telemetry.persist_failed recorded on the run");
    match &recorded.kind {
        V2AuditEventKind::TelemetryPersistFailed {
            component, step_id, ..
        } => {
            assert_eq!(component, "invocation_trace");
            assert_eq!(step_id.as_deref(), Some("implement_one"));
        }
        other => panic!("unexpected event kind: {other:?}"),
    }

    // The audit trail itself is undamaged — this is a telemetry gap only.
    assert_eq!(ctx.audit.audit_failure_count(), 0);
}
