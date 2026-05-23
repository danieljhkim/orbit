#![allow(missing_docs)]

use super::*;

use orbit_common::types::activity_job::{AgentLoopSpec, AgentRole, Backend, OnDenial, Provider};

use crate::AgentRoleConfig;

use super::role_overridden_recovery_spec;

#[test]
fn recovery_success_runs_one_post_recovery_attempt_with_exact_input_and_fs_profile() {
    let original_error = retryable_error("flaky", "dirty checkout");
    let host = RecoveryHost::new([
        (
            "flaky",
            vec![
                Err(original_error.clone()),
                Err(original_error.clone()),
                Ok(json!({"fixed": true})),
            ],
        ),
        ("recover", vec![Ok(json!({"recovered": true}))]),
    ]);
    let job = recovery_job(Some("recover"), Some("wide"), "flaky", Some("narrow"), 2);
    let writer = std::sync::Arc::new(test_writer("run-recovery-success"));

    let outcome = execute_job(
        &job,
        Value::Null,
        "run-recovery-success",
        writer.clone(),
        &host,
    )
    .expect("job should recover");

    assert!(outcome.success);
    assert_eq!(host.actions(), vec!["flaky", "flaky", "recover", "flaky"]);
    assert_eq!(host.action_count("recover"), 1);
    assert_eq!(
        host.input_for_action("recover"),
        Some(json!({
            "failed_step_id": "build",
            "activity_name": "flaky",
            "error_message": original_error.to_string(),
            "attempt": 2,
            "max_attempts": 2
        }))
    );
    assert_eq!(
        host.fs_profile_for_action("recover"),
        Some(Some("narrow".to_string()))
    );

    let events = writer.events_snapshot().expect("audit snapshot");
    let recovery_events = recovery_events(&events);
    assert_eq!(recovery_events.len(), 1);
    assert!(matches!(
        recovery_events[0].kind,
        V2AuditEventKind::StepRecoveryAttempted {
            ref step_id,
            ref recovery_activity,
            recovery_succeeded: true,
        } if step_id == "build" && recovery_activity == "recover"
    ));
}

#[test]
fn recovery_success_with_post_recovery_failure_returns_original_error_text() {
    let original_error = retryable_error("flaky", "first failure");
    let post_recovery_error = retryable_error("flaky", "post recovery still failing");
    let host = RecoveryHost::new([
        (
            "flaky",
            vec![
                Err(original_error.clone()),
                Err(original_error.clone()),
                Err(post_recovery_error),
            ],
        ),
        ("recover", vec![Ok(json!({"recovered": true}))]),
    ]);
    let job = recovery_job(Some("recover"), None, "flaky", None, 2);
    let writer = std::sync::Arc::new(test_writer("run-post-recovery-failure"));

    let err = execute_job(
        &job,
        Value::Null,
        "run-post-recovery-failure",
        writer.clone(),
        &host,
    )
    .expect_err("post-recovery failure should surface original error");

    assert_eq!(err.to_string(), original_error.to_string());
    assert_eq!(host.action_count("recover"), 1);
    assert_eq!(recovery_events(&writer.events_snapshot().unwrap()).len(), 1);
}

#[test]
fn recovery_activity_error_returns_original_error_text() {
    let original_error = retryable_error("flaky", "precondition failed");
    let host = RecoveryHost::new([
        ("flaky", vec![Err(original_error.clone())]),
        (
            "recover",
            vec![Err(retryable_error("recover", "could not fix"))],
        ),
    ]);
    let job = recovery_job(Some("recover"), None, "flaky", None, 1);
    let writer = std::sync::Arc::new(test_writer("run-recovery-error"));

    let err = execute_job(
        &job,
        Value::Null,
        "run-recovery-error",
        writer.clone(),
        &host,
    )
    .expect_err("recovery error should surface original error");

    assert_eq!(err.to_string(), original_error.to_string());
    assert_eq!(host.action_count("recover"), 1);
    let events = writer.events_snapshot().expect("audit snapshot");
    assert!(matches!(
        recovery_events(&events)[0].kind,
        V2AuditEventKind::StepRecoveryAttempted {
            recovery_succeeded: false,
            ..
        }
    ));
}

#[test]
fn step_level_recovery_activity_runs_without_job_level_recovery() {
    let original_error = retryable_error("flaky", "dirty checkout");
    let host = RecoveryHost::new([
        (
            "flaky",
            vec![Err(original_error.clone()), Ok(json!({"fixed": true}))],
        ),
        ("recover_step", vec![Ok(json!({"recovered": true}))]),
    ]);
    let mut job = recovery_job(None, None, "flaky", Some("narrow"), 1);
    job.steps[0].recovery_activity = Some("recover_step".to_string());
    job.steps[0].resolved_recovery_activity =
        Some(deterministic_activity("recover_step", Some("wide")));
    let writer = std::sync::Arc::new(test_writer("run-step-recovery-success"));

    let outcome = execute_job(
        &job,
        Value::Null,
        "run-step-recovery-success",
        writer.clone(),
        &host,
    )
    .expect("job should recover through step-level activity");

    assert!(outcome.success);
    assert_eq!(host.actions(), vec!["flaky", "recover_step", "flaky"]);
    assert_eq!(host.action_count("recover_step"), 1);
    assert_eq!(
        host.fs_profile_for_action("recover_step"),
        Some(Some("narrow".to_string()))
    );
    let events = writer.events_snapshot().expect("audit snapshot");
    assert!(matches!(
        recovery_events(&events)[0].kind,
        V2AuditEventKind::StepRecoveryAttempted {
            ref recovery_activity,
            ..
        } if recovery_activity == "recover_step"
    ));
}

#[test]
fn recovery_agent_loop_uses_reviewer_role_config() {
    let mut role_config = HashMap::new();
    role_config.insert(
        AgentRole::Reviewer,
        AgentRoleConfig {
            provider: Some(Provider::Gemini),
            model: Some("gemini-3.1-pro".to_string()),
            backend: Some(Backend::Cli),
        },
    );
    let host = RecoveryHost::empty().with_role_config(role_config);
    let ctx = recovery_exec_ctx(&host);
    let recovery = agent_loop_recovery_activity(recovery_agent_loop_spec(
        Some(AgentRole::Reviewer),
        Provider::Claude,
        Backend::Http,
        None,
    ));

    let overridden = role_overridden_recovery_spec(&recovery, &ctx)
        .expect("role-tagged recovery should resolve");

    let ActivityV2Spec::AgentLoop(spec) = overridden else {
        panic!("expected agent_loop recovery spec");
    };
    assert_eq!(spec.provider, Provider::Gemini);
    assert_eq!(spec.model.as_deref(), Some("gemini-3.1-pro"));
    assert_eq!(spec.backend, Backend::Cli);
    assert_eq!(host.observed_role_lookups(), vec![AgentRole::Reviewer]);
}

#[test]
fn recovery_agent_loop_without_reviewer_config_keeps_inline_defaults() {
    let host = RecoveryHost::empty();
    let ctx = recovery_exec_ctx(&host);
    let recovery = agent_loop_recovery_activity(recovery_agent_loop_spec(
        Some(AgentRole::Reviewer),
        Provider::Claude,
        Backend::Http,
        None,
    ));

    let overridden = role_overridden_recovery_spec(&recovery, &ctx)
        .expect("role-tagged recovery should still produce dispatch spec");

    let ActivityV2Spec::AgentLoop(spec) = overridden else {
        panic!("expected agent_loop recovery spec");
    };
    assert_eq!(spec.provider, Provider::Claude);
    assert_eq!(spec.model, None);
    assert_eq!(spec.backend, Backend::Http);
    assert_eq!(host.observed_role_lookups(), vec![AgentRole::Reviewer]);
}

#[test]
fn unknown_step_recovery_activity_name_is_job_validation_during_catalog_resolution() {
    let yaml = r#"
schemaVersion: 2
kind: Job
metadata:
  name: missing_step_recovery
spec:
  state: enabled
  steps:
    - id: build
      recovery_activity: missing
      spec:
        type: deterministic
        action: flaky
"#;
    let mut job = load_job_asset(yaml).expect("job yaml").spec;
    let catalog = V2ActivityCatalog::new();

    let err = resolve_job_catalog_refs_for_execution(&mut job, &catalog)
        .expect_err("missing step recovery activity should fail resolution");

    assert!(matches!(
        err,
        DispatchError::JobValidation(ref message)
            if message.contains("step `build`: recovery_activity `missing` not found")
    ));
}

#[test]
fn non_retryable_failure_skips_recovery_and_audit_event() {
    let host = RecoveryHost::new([
        (
            "flaky",
            vec![Err(DispatchError::ToolDenied {
                tool_name: "fs.write".to_string(),
                iteration: 1,
            })],
        ),
        ("recover", vec![Ok(json!({"recovered": true}))]),
    ]);
    let job = recovery_job(Some("recover"), None, "flaky", None, 2);
    let writer = std::sync::Arc::new(test_writer("run-non-retryable"));

    let err = execute_job(
        &job,
        Value::Null,
        "run-non-retryable",
        writer.clone(),
        &host,
    )
    .expect_err("tool denial should bypass recovery");

    assert!(matches!(err, DispatchError::ToolDenied { .. }));
    assert_eq!(host.action_count("recover"), 0);
    assert!(recovery_events(&writer.events_snapshot().unwrap()).is_empty());
}

#[test]
fn no_recovery_activity_preserves_success_and_failure_paths() {
    let original_error = retryable_error("flaky", "still failing");
    let failing_host = RecoveryHost::new([("flaky", vec![Err(original_error.clone())])]);
    let failing_job = recovery_job(None, None, "flaky", None, 1);
    let failing_writer = std::sync::Arc::new(test_writer("run-no-recovery-failure"));

    let err = execute_job(
        &failing_job,
        Value::Null,
        "run-no-recovery-failure",
        failing_writer.clone(),
        &failing_host,
    )
    .expect_err("retryable failure should remain the original error");

    assert_eq!(err.to_string(), original_error.to_string());
    assert!(recovery_events(&failing_writer.events_snapshot().unwrap()).is_empty());

    let success_host = RecoveryHost::new([("stable", vec![Ok(json!({"ok": true}))])]);
    let success_job = recovery_job(None, None, "stable", None, 1);
    let success_writer = std::sync::Arc::new(test_writer("run-no-recovery-success"));

    let outcome = execute_job(
        &success_job,
        Value::Null,
        "run-no-recovery-success",
        success_writer.clone(),
        &success_host,
    )
    .expect("success path should remain unchanged");

    assert!(outcome.success);
    assert!(recovery_events(&success_writer.events_snapshot().unwrap()).is_empty());
}

#[test]
fn unknown_recovery_activity_name_is_job_validation_during_catalog_resolution() {
    let yaml = r#"
schemaVersion: 2
kind: Job
metadata:
  name: missing_recovery
spec:
  state: enabled
  recovery_activity: missing
  steps:
    - id: build
      spec:
        type: deterministic
        action: flaky
"#;
    let mut job = load_job_asset(yaml).expect("job yaml").spec;
    let catalog = V2ActivityCatalog::new();

    let err = resolve_job_catalog_refs_for_execution(&mut job, &catalog)
        .expect_err("missing recovery activity should fail resolution");

    assert!(matches!(
        err,
        DispatchError::JobValidation(ref message)
            if message.contains("recovery_activity `missing` not found")
    ));
}

fn recovery_job(
    recovery_name: Option<&str>,
    recovery_fs_profile: Option<&str>,
    step_action: &str,
    step_fs_profile: Option<&str>,
    max_attempts: u32,
) -> JobV2 {
    JobV2 {
        state: JobScheduleState::Enabled,
        default_input: None,
        recovery_activity: recovery_name.map(str::to_string),
        resolved_recovery_activity: recovery_name
            .map(|name| deterministic_activity(name, recovery_fs_profile)),
        max_active_runs: 1,
        kind: JobKind::Workflow,
        steps: vec![JobV2Step {
            id: "build".to_string(),
            when: None,
            retry: Some(RetrySpec {
                max_attempts,
                initial_backoff_ms: 0,
                backoff_cap_ms: 0,
                backoff_strategy: BackoffStrategy::Linear,
            }),
            recovery_activity: None,
            resolved_recovery_activity: None,
            body: JobV2StepBody::Target(TargetStep {
                spec: deterministic_activity(step_action, None).spec,
                activity_name: None,
                fs_profile: step_fs_profile.map(str::to_string),
                default_input: None,
                timeout_seconds: 0,
                session: None,
                role: None,
            }),
        }],
    }
}

fn deterministic_activity(action: &str, fs_profile: Option<&str>) -> ActivityV2 {
    ActivityV2 {
        description: format!("deterministic {action}"),
        input_schema_json: json!({}),
        output_schema_json: json!({}),
        fs_profile: fs_profile.map(str::to_string),
        spec: ActivityV2Spec::Deterministic(DeterministicSpec {
            action: action.to_string(),
            config: Value::Null,
        }),
    }
}

fn agent_loop_recovery_activity(spec: AgentLoopSpec) -> ResolvedRecoveryActivity {
    ResolvedRecoveryActivity {
        name: "recover".to_string(),
        spec: ActivityV2Spec::AgentLoop(spec),
    }
}

fn recovery_agent_loop_spec(
    role: Option<AgentRole>,
    provider: Provider,
    backend: Backend,
    model: Option<&str>,
) -> AgentLoopSpec {
    AgentLoopSpec {
        instruction: "recover carefully".to_string(),
        tools: Vec::new(),
        on_denial: OnDenial::Terminate,
        model: model.map(str::to_string),
        max_iterations: 1,
        backend,
        provider,
        wall_clock_timeout_seconds: 30,
        role,
        proc_allowed_programs: None,
    }
}

fn recovery_exec_ctx<'a>(host: &'a dyn V2RuntimeHost) -> ExecCtx<'a> {
    ExecCtx {
        run_id: "run-recovery-role".to_string(),
        audit: std::sync::Arc::new(test_writer("run-recovery-role")),
        host,
        input: json!({}),
        pipeline: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
        sessions: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
        recovery_activity: None,
        item: None,
        iteration: None,
    }
}

fn retryable_error(action: &str, message: &str) -> DispatchError {
    DispatchError::DeterministicActionFailed {
        action: action.to_string(),
        message: message.to_string(),
    }
}

fn recovery_events(events: &[V2AuditEvent]) -> Vec<&V2AuditEvent> {
    events
        .iter()
        .filter(|event| matches!(event.kind, V2AuditEventKind::StepRecoveryAttempted { .. }))
        .collect()
}

#[derive(Debug, Clone)]
struct DeterministicCall {
    action: String,
    input: Value,
    fs_profile: Option<String>,
}

struct RecoveryHost {
    responses: StdMutex<HashMap<String, VecDeque<Result<Value, DispatchError>>>>,
    calls: StdMutex<Vec<DeterministicCall>>,
    pending_fs_profiles: StdMutex<VecDeque<Option<String>>>,
    role_config: StdMutex<HashMap<AgentRole, AgentRoleConfig>>,
    observed_role_lookups: StdMutex<Vec<AgentRole>>,
}

impl RecoveryHost {
    fn empty() -> Self {
        Self::new([])
    }

    fn new<const N: usize>(responses: [(&str, Vec<Result<Value, DispatchError>>); N]) -> Self {
        Self {
            responses: StdMutex::new(
                responses
                    .into_iter()
                    .map(|(action, outcomes)| (action.to_string(), outcomes.into_iter().collect()))
                    .collect(),
            ),
            calls: StdMutex::new(Vec::new()),
            pending_fs_profiles: StdMutex::new(VecDeque::new()),
            role_config: StdMutex::new(HashMap::new()),
            observed_role_lookups: StdMutex::new(Vec::new()),
        }
    }

    fn with_role_config(self, config: HashMap<AgentRole, AgentRoleConfig>) -> Self {
        *self.role_config.lock().expect("role config lock") = config;
        self
    }

    fn actions(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("calls lock")
            .iter()
            .map(|call| call.action.clone())
            .collect()
    }

    fn action_count(&self, action: &str) -> usize {
        self.calls
            .lock()
            .expect("calls lock")
            .iter()
            .filter(|call| call.action == action)
            .count()
    }

    fn input_for_action(&self, action: &str) -> Option<Value> {
        self.calls
            .lock()
            .expect("calls lock")
            .iter()
            .find(|call| call.action == action)
            .map(|call| call.input.clone())
    }

    fn fs_profile_for_action(&self, action: &str) -> Option<Option<String>> {
        self.calls
            .lock()
            .expect("calls lock")
            .iter()
            .find(|call| call.action == action)
            .map(|call| call.fs_profile.clone())
    }

    fn observed_role_lookups(&self) -> Vec<AgentRole> {
        self.observed_role_lookups
            .lock()
            .expect("observed role lock")
            .clone()
    }
}

impl V2RuntimeHost for RecoveryHost {
    fn run_deterministic(
        &self,
        action: &str,
        _config: &Value,
        input: &Value,
        _tool_context: orbit_tools::ToolContext,
    ) -> Result<Value, DispatchError> {
        let fs_profile = self
            .pending_fs_profiles
            .lock()
            .expect("fs profiles lock")
            .pop_front()
            .unwrap_or(None);
        self.calls
            .lock()
            .expect("calls lock")
            .push(DeterministicCall {
                action: action.to_string(),
                input: input.clone(),
                fs_profile,
            });

        self.responses
            .lock()
            .expect("responses lock")
            .get_mut(action)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| Ok(json!({"action": action})))
    }

    fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::AgentLoopFailed(
            "test host: no credentials".into(),
        ))
    }

    fn resolve_cli_executor(
        &self,
        _provider: &str,
    ) -> Result<super::super::super::dispatcher::ResolvedCliExecutor, DispatchError> {
        Err(DispatchError::CliInvocationFailed(
            "test host: no CLI mapping".into(),
        ))
    }

    fn tool_context_for_activity(
        &self,
        _run_id: Option<&str>,
        fs_profile: Option<&str>,
        _fs_audit: Option<std::sync::Arc<dyn orbit_tools::FsAuditLogger>>,
        _proc_allowed_programs: Option<&[String]>,
    ) -> orbit_tools::ToolContext {
        self.pending_fs_profiles
            .lock()
            .expect("fs profiles lock")
            .push_back(fs_profile.map(str::to_string));
        orbit_tools::ToolContext::default()
    }

    fn agent_role_config(&self, role: AgentRole) -> Option<AgentRoleConfig> {
        self.observed_role_lookups
            .lock()
            .expect("observed role lock")
            .push(role);
        self.role_config
            .lock()
            .expect("role config lock")
            .get(&role)
            .cloned()
    }
}
