#![allow(missing_docs)]

use super::*;

use orbit_common::test_fixtures::TEST_CLAUDE_MODEL;
use orbit_types::workflow::activity_job::{AgentLoopSpec, OnDenial, Provider};
use std::sync::Mutex;

use crate::CrewConfig;

use super::crew_overridden_spec;

struct CrewHost {
    config: HashMap<String, CrewConfig>,
    observed: Mutex<Vec<String>>,
}

impl CrewHost {
    fn new(config: impl IntoIterator<Item = (&'static str, CrewConfig)>) -> Self {
        Self {
            config: config
                .into_iter()
                .map(|(name, config)| (name.to_string(), config))
                .collect(),
            observed: Mutex::new(Vec::new()),
        }
    }

    fn observed(&self) -> Vec<String> {
        self.observed.lock().expect("observed lock").clone()
    }
}

impl RuntimeHost for CrewHost {
    fn run_deterministic(
        &self,
        _action: &str,
        _config: &Value,
        _input: &Value,
        _tool_context: orbit_tools::ToolContext,
    ) -> Result<Value, DispatchError> {
        Err(DispatchError::DeterministicActionNotRegistered(
            "crew host: not used".into(),
        ))
    }

    fn resolve_cli_executor(
        &self,
        _provider: &str,
    ) -> Result<super::super::super::dispatcher::ResolvedCliExecutor, DispatchError> {
        Err(DispatchError::CliInvocationFailed(
            "crew host: no CLI mapping".into(),
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

    fn agent_crew_config_for_input(
        &self,
        input: &Value,
    ) -> Result<Option<CrewConfig>, DispatchError> {
        let name = input
            .get("crew")
            .and_then(Value::as_str)
            .unwrap_or("run-default");
        self.observed
            .lock()
            .expect("observed lock")
            .push(name.to_string());
        self.config
            .get(name)
            .cloned()
            .map(Some)
            .ok_or_else(|| DispatchError::JobValidation(format!("test crew `{name}` is unknown")))
    }
}

fn inline_agent_loop_spec() -> AgentLoopSpec {
    AgentLoopSpec {
        instruction: "inline".to_string(),
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

fn target_step(spec: ActivityV2Spec) -> TargetStep {
    TargetStep {
        spec,
        activity_name: None,
        fs_profile: None,
        default_input: None,
        timeout_seconds: 0,
        session: None,
    }
}

fn exec_ctx<'a>(host: &'a dyn RuntimeHost) -> ExecCtx<'a> {
    ExecCtx {
        run_id: "run-crew-override".to_string(),
        audit: std::sync::Arc::new(test_writer("run-crew-override")),
        host,
        input: json!({ "crew": "run-default" }),
        pipeline: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
        recovery_activity: None,
        failure_activity: None,
        item: None,
        iteration: None,
    }
}

fn config(provider: Provider, model: &str) -> CrewConfig {
    CrewConfig {
        provider: Some(provider),
        model: Some(model.to_string()),
    }
}

#[test]
fn explicit_activity_crew_routes_to_that_crew() {
    let host = CrewHost::new([
        ("run-default", config(Provider::Claude, "run-model")),
        ("activity", config(Provider::Codex, "activity-model")),
    ]);
    let ctx = exec_ctx(&host);
    let target = target_step(ActivityV2Spec::AgentLoop(inline_agent_loop_spec()));

    let overridden = crew_overridden_spec(&target, &ctx, &json!({ "crew": "activity" }))
        .expect("resolve explicit crew")
        .expect("configured host returns an override");
    assert_eq!(overridden.provider, Provider::Codex);
    assert_eq!(overridden.model.as_deref(), Some("activity-model"));
    assert_eq!(host.observed(), vec!["activity"]);
}

#[test]
fn activity_without_crew_routes_to_run_resolved_crew() {
    let host = CrewHost::new([("run-default", config(Provider::Codex, "run-resolved-model"))]);
    let ctx = exec_ctx(&host);
    let target = target_step(ActivityV2Spec::AgentLoop(inline_agent_loop_spec()));

    let overridden = crew_overridden_spec(&target, &ctx, &json!({}))
        .expect("resolve run fallback")
        .expect("run crew must override the inline baseline");
    assert_eq!(overridden.provider, Provider::Codex);
    assert_eq!(overridden.model.as_deref(), Some("run-resolved-model"));
    assert_ne!(overridden.model.as_deref(), Some(TEST_CLAUDE_MODEL));
    assert_eq!(host.observed(), vec!["run-default"]);
}

#[test]
fn explicit_unknown_crew_fails_closed() {
    let host = CrewHost::new([("run-default", config(Provider::Claude, "run-model"))]);
    let ctx = exec_ctx(&host);
    let target = target_step(ActivityV2Spec::AgentLoop(inline_agent_loop_spec()));

    let error = crew_overridden_spec(&target, &ctx, &json!({ "crew": "unknown" }))
        .expect_err("unknown explicit crew must fail");
    assert!(matches!(error, DispatchError::JobValidation(_)));
}

#[test]
fn crew_resolution_does_not_apply_to_deterministic_specs() {
    let host = CrewHost::new([("run-default", config(Provider::Claude, "run-model"))]);
    let ctx = exec_ctx(&host);
    let target = target_step(ActivityV2Spec::Deterministic(DeterministicSpec {
        action: "noop".to_string(),
        config: Value::Null,
    }));

    assert!(
        crew_overridden_spec(&target, &ctx, &ctx.input)
            .expect("deterministic target is unaffected")
            .is_none()
    );
    assert!(host.observed().is_empty());
}

// ----- Telemetry persistence is non-fatal (ORB-10367) -----------------

struct FailingTelemetryHost;

impl RuntimeHost for FailingTelemetryHost {
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
        _trace: &orbit_types::telemetry::InvocationTrace,
    ) -> Result<(), DispatchError> {
        Err(DispatchError::JobExecution(
            "persist invocation trace: store error: table invocations has no column named cache_create_1h_tokens"
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
            trace: orbit_types::telemetry::InvocationTrace::default(),
        }),
    }
}

#[test]
fn failed_invocation_trace_persist_does_not_fail_the_step() {
    let host = FailingTelemetryHost;
    let ctx = exec_ctx(&host);
    let dispatch = dispatch_with_invocation();

    let trace = capture(|| {
        super::persist_dispatch_invocation(&ctx, "implement_one", &ctx.input, &dispatch);
    });

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
            .contains("cache_create_1h_tokens")
    );

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
    assert_eq!(ctx.audit.audit_failure_count(), 0);
}
