use std::io::Write;

use chrono::{DateTime, Utc};
use orbit_agent::{
    Agent, AgentConfig, AgentRequest, AgentResponseStatus, parse_and_validate_response,
};
use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_store::ClaimedJobRun;
use orbit_store::JobCreateParams as StoreActivityCreateParams;
use orbit_store::JobRunCompletionParams;
use orbit_types::{
    Activity, AgentCommitRequest, AgentResponseEnvelope, Job, JobRetryBackoffStrategy, JobRun,
    JobRunState, JobScheduleState, JobTargetType, OrbitError, OrbitEvent,
};
use serde::Serialize;
use serde_json::{Value, json};
use tempfile::NamedTempFile;

use crate::OrbitRuntime;
use crate::json_schema::validate_instance_against_schema;
use crate::paths;
const AGENT_PROTOCOL_VIOLATION: &str = "AGENT_PROTOCOL_VIOLATION";
const AGENT_INVOCATION_FAILED: &str = "AGENT_INVOCATION_FAILED";
const AGENT_COMMIT_FAILED: &str = "AGENT_COMMIT_FAILED";
const AGENT_TIMEOUT: &str = "AGENT_TIMEOUT";
const STALE_RUN_GRACE_SECONDS: u64 = 30;

#[derive(Debug, Clone)]
pub struct JobAddParams {
    pub job_id: Option<String>,
    pub target_type: JobTargetType,
    pub target_id: String,
    pub schedule: String,
    pub agent_cli: String,
    pub timeout_seconds: u64,
    pub retry_max_attempts: u32,
    pub retry_backoff_strategy: JobRetryBackoffStrategy,
    pub retry_initial_delay_seconds: u64,
    pub initial_state_override: Option<JobScheduleState>,
}

#[derive(Debug, Clone)]
pub struct JobRunResult {
    pub job_id: String,
    pub run_id: String,
    pub state: JobRunState,
    pub attempt: u32,
}

#[derive(Debug, Clone)]
struct AttemptOutcome {
    state: JobRunState,
    exit_code: Option<i32>,
    duration_ms: Option<u64>,
    response_json: Option<Value>,
    error_code: Option<String>,
    error_message: Option<String>,
    retryable: bool,
    protocol_violation: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ExecutionEnvelope {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    activity: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    job: Option<Value>,
    skills: Vec<ExecutionSkillEnvelope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identity: Option<Value>,
    input: Value,
    memory: Value,
}

#[derive(Debug, Clone, Serialize)]
struct ExecutionSkillEnvelope {
    id: String,
    content_hash: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<Value>,
}

#[derive(Debug, Clone)]
struct ExecutionContext {
    activity: Activity,
    job: Option<Job>,
    agent_cli: String,
    timeout_seconds: u64,
    input: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct DirectActivityRunOutcome {
    pub(crate) state: JobRunState,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) error_code: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) protocol_violation: bool,
}

impl OrbitRuntime {
    pub fn add_job(&self, params: JobAddParams) -> Result<Job, OrbitError> {
        if params.target_id.trim().is_empty() {
            return Err(OrbitError::JobValidation(
                "target_id must not be empty".to_string(),
            ));
        }
        if params.schedule.trim().is_empty() {
            return Err(OrbitError::JobValidation(
                "schedule must not be empty".to_string(),
            ));
        }
        if params.agent_cli.trim().is_empty() {
            return Err(OrbitError::JobValidation(
                "agent_cli must not be empty".to_string(),
            ));
        }

        self.validate_activity_target_exists(params.target_type, &params.target_id)?;

        // Validate runtime availability at add-time.
        let _ = Agent::new(&AgentConfig::cli(params.agent_cli.clone()))?;

        let is_manual = params.schedule.trim().eq_ignore_ascii_case("manual");
        let (next_run_at, initial_state) = if is_manual {
            // Manual jobs never auto-fire; use a far-future sentinel for scheduler bookkeeping.
            let far_future = manual_job_next_run_at();
            (
                far_future,
                params
                    .initial_state_override
                    .unwrap_or(JobScheduleState::Disabled),
            )
        } else {
            let next_run_at =
                crate::job::state_machine::compute_next_run_at(&params.schedule, Utc::now())?;
            (
                next_run_at,
                params
                    .initial_state_override
                    .unwrap_or(JobScheduleState::Enabled),
            )
        };

        let job = self.context.job_store.add_job(StoreActivityCreateParams {
            job_id: params.job_id,
            target_type: params.target_type,
            target_id: params.target_id,
            schedule: params.schedule,
            agent_cli: params.agent_cli,
            timeout_seconds: params.timeout_seconds,
            retry_max_attempts: params.retry_max_attempts,
            retry_backoff_strategy: params.retry_backoff_strategy,
            retry_initial_delay_seconds: params.retry_initial_delay_seconds,
            next_run_at,
            initial_state,
        })?;
        self.record_event(OrbitEvent::JobAdded {
            job_id: job.job_id.clone(),
        })?;
        Ok(job)
    }

    pub fn list_jobs(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        self.list_jobs_backend(include_disabled)
    }

    pub fn show_job(&self, job_id: &str) -> Result<Job, OrbitError> {
        self.get_job_backend(job_id)?
            .ok_or_else(|| OrbitError::JobNotFound(job_id.to_string()))
    }

    pub fn pause_job(&self, job_id: &str) -> Result<(), OrbitError> {
        let _ = self.show_job(job_id)?;
        let changed = self
            .context
            .job_store
            .set_job_state(job_id, JobScheduleState::Paused)?;
        if !changed {
            return Err(OrbitError::JobNotFound(job_id.to_string()));
        }
        self.record_event(OrbitEvent::JobPaused {
            job_id: job_id.to_string(),
        })
    }

    pub fn resume_job(&self, job_id: &str) -> Result<(), OrbitError> {
        let job = self.show_job(job_id)?;
        let next_run_at = if job.schedule.trim().eq_ignore_ascii_case("manual") {
            manual_job_next_run_at()
        } else {
            crate::job::state_machine::compute_next_run_at(&job.schedule, Utc::now())?
        };

        let changed = self
            .context
            .job_store
            .set_job_state(job_id, JobScheduleState::Enabled)?;
        if !changed {
            return Err(OrbitError::JobNotFound(job_id.to_string()));
        }
        let _ = self
            .context
            .job_store
            .update_job_next_run(job_id, next_run_at)?;
        self.record_event(OrbitEvent::JobResumed {
            job_id: job_id.to_string(),
        })
    }

    pub fn delete_job(&self, job_id: &str) -> Result<(), OrbitError> {
        let changed = self.context.job_store.mark_job_disabled(job_id)?;
        if !changed {
            return Err(OrbitError::JobNotFound(job_id.to_string()));
        }
        self.record_event(OrbitEvent::JobDeleted {
            job_id: job_id.to_string(),
        })
    }

    pub fn run_job_now(&self, job_id: &str) -> Result<JobRunResult, OrbitError> {
        self.run_job_now_with_input(job_id, json!({}))
    }

    pub fn run_job_now_with_input(
        &self,
        job_id: &str,
        input: Value,
    ) -> Result<JobRunResult, OrbitError> {
        let job = self.show_job(job_id)?;
        let _ = self.recover_stale_active_run_for_job(&job, Utc::now())?;
        if let Some(active_run) = self.get_pending_or_running_job_run_backend(job_id)? {
            return Err(OrbitError::JobValidation(format!(
                "job '{}' already has an active run '{}' in state '{}'",
                job_id, active_run.run_id, active_run.state
            )));
        }
        self.record_event(OrbitEvent::JobTriggered {
            job_id: job.job_id.clone(),
        })?;

        self.execute_activity_with_retries(job, Utc::now(), None, input)
    }

    pub(crate) fn execute_claimed_job(&self, claimed: &ClaimedJobRun) -> Result<(), OrbitError> {
        let _ = self.execute_activity_with_retries(
            claimed.job.clone(),
            claimed.run.scheduled_at,
            Some(claimed.run.clone()),
            json!({}),
        )?;
        Ok(())
    }

    fn execute_activity_with_retries(
        &self,
        job: Job,
        scheduled_at: DateTime<Utc>,
        initial_run: Option<JobRun>,
        input: Value,
    ) -> Result<JobRunResult, OrbitError> {
        let execution = self.build_execution_context_for_job(&job, input)?;
        let max_attempts = job.retry_max_attempts.saturating_add(1);
        let mut current_attempt = initial_run.as_ref().map(|r| r.attempt).unwrap_or(1);
        let mut pending_initial = initial_run;
        let mut last_result: Option<JobRunResult> = None;
        let mut retry_scheduled_for_future = false;

        while current_attempt <= max_attempts {
            let mut run = if let Some(existing) = pending_initial.take() {
                existing
            } else {
                let run =
                    self.insert_job_run_backend(&job.job_id, current_attempt, scheduled_at)?;
                self.record_event(OrbitEvent::JobRunStarted {
                    job_id: job.job_id.clone(),
                    run_id: String::new(),
                    attempt: current_attempt,
                })?;
                run
            };

            let started_at = Utc::now();
            let changed = self.mark_job_run_running_backend(&run.run_id, started_at)?;
            if !changed {
                return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
            }
            self.record_event(OrbitEvent::JobRunStarted {
                job_id: job.job_id.clone(),
                run_id: run.run_id.clone(),
                attempt: run.attempt,
            })?;
            run.state = JobRunState::Running;
            run.started_at = Some(started_at);

            let outcome = self.execute_single_attempt(&execution);
            let finished_at = Utc::now();

            let changed = self.complete_job_run_backend(&JobRunCompletionParams {
                run_id: &run.run_id,
                state: outcome.state,
                finished_at,
                duration_ms: outcome.duration_ms,
                exit_code: outcome.exit_code,
                agent_response_json: outcome.response_json.as_ref(),
                error_code: outcome.error_code.as_deref(),
                error_message: outcome.error_message.as_deref(),
            })?;
            if !changed {
                return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
            }
            self.record_event(OrbitEvent::JobRunCompleted {
                job_id: job.job_id.clone(),
                run_id: run.run_id.clone(),
                state: outcome.state.to_string(),
            })?;

            if outcome.protocol_violation {
                self.record_event(OrbitEvent::JobProtocolViolation {
                    job_id: job.job_id.clone(),
                    run_id: run.run_id.clone(),
                    message: outcome
                        .error_message
                        .clone()
                        .unwrap_or_else(|| "agent protocol violation".to_string()),
                })?;
            }

            last_result = Some(JobRunResult {
                job_id: job.job_id.clone(),
                run_id: run.run_id.clone(),
                state: outcome.state,
                attempt: run.attempt,
            });

            if outcome.state == JobRunState::Success {
                break;
            }

            if outcome.retryable && current_attempt < max_attempts {
                let retry_index = current_attempt;
                let delay_seconds = crate::job::state_machine::compute_retry_delay_seconds(
                    job.retry_backoff_strategy,
                    job.retry_initial_delay_seconds,
                    retry_index,
                );
                let next_retry_at = Utc::now() + chrono::Duration::seconds(delay_seconds as i64);

                let _ = self.update_job_next_run_backend(&job.job_id, next_retry_at)?;
                self.record_event(OrbitEvent::JobRetryScheduled {
                    job_id: job.job_id.clone(),
                    run_id: run.run_id.clone(),
                    next_run_at: next_retry_at.to_rfc3339(),
                })?;

                if delay_seconds > 0 {
                    // Avoid blocking job execution while waiting for delayed retries.
                    retry_scheduled_for_future = true;
                    break;
                }

                current_attempt = current_attempt.saturating_add(1);
                continue;
            }

            break;
        }

        if !retry_scheduled_for_future {
            let next_run_at = if job.schedule.trim().eq_ignore_ascii_case("manual") {
                manual_job_next_run_at()
            } else {
                crate::job::state_machine::compute_next_run_at(&job.schedule, Utc::now())?
            };
            let _ = self.update_job_next_run_backend(&job.job_id, next_run_at);
            let _ = self.record_event(OrbitEvent::JobTriggered {
                job_id: job.job_id.clone(),
            });
        }

        last_result.ok_or(OrbitError::JobRunNotFound(job.job_id))
    }

    pub(crate) fn recover_stale_active_run_for_job(
        &self,
        job: &Job,
        now: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        let Some(active_run) = self.get_pending_or_running_job_run_backend(&job.job_id)? else {
            return Ok(false);
        };

        if !is_stale_active_run(job, &active_run, now) {
            return Ok(false);
        }

        let reference_time = active_run.started_at.unwrap_or(active_run.created_at);
        let age_seconds = now
            .signed_duration_since(reference_time)
            .num_seconds()
            .max(0) as u64;
        let duration_ms = active_run
            .started_at
            .map(|started| now.signed_duration_since(started).num_milliseconds().max(0) as u64);
        let message = format!(
            "stale active run recovered: run '{}' remained '{}' for {}s \
(timeout={}s, grace={}s)",
            active_run.run_id,
            active_run.state,
            age_seconds,
            job.timeout_seconds,
            STALE_RUN_GRACE_SECONDS
        );

        let changed = self.complete_job_run_backend(&JobRunCompletionParams {
            run_id: &active_run.run_id,
            state: JobRunState::Failed,
            finished_at: now,
            duration_ms,
            exit_code: Some(1),
            agent_response_json: None,
            error_code: Some(AGENT_INVOCATION_FAILED),
            error_message: Some(&message),
        })?;
        if !changed {
            return Err(OrbitError::JobRunNotFound(active_run.run_id.clone()));
        }
        self.record_event(OrbitEvent::JobRunCompleted {
            job_id: job.job_id.clone(),
            run_id: active_run.run_id.clone(),
            state: JobRunState::Failed.to_string(),
        })?;

        Ok(true)
    }

    pub(crate) fn run_activity_direct(
        &self,
        activity: &Activity,
        agent_cli: &str,
        timeout_seconds: u64,
    ) -> Result<DirectActivityRunOutcome, OrbitError> {
        let execution = ExecutionContext {
            activity: activity.clone(),
            job: None,
            agent_cli: agent_cli.to_string(),
            timeout_seconds,
            input: json!({}),
        };
        let outcome = self.execute_single_attempt(&execution);
        Ok(DirectActivityRunOutcome {
            state: outcome.state,
            duration_ms: outcome.duration_ms,
            error_code: outcome.error_code,
            error_message: outcome.error_message,
            protocol_violation: outcome.protocol_violation,
        })
    }

    fn build_execution_context_for_job(
        &self,
        job: &Job,
        input: Value,
    ) -> Result<ExecutionContext, OrbitError> {
        let activity = self.show_activity(&job.target_id)?;
        self.validate_activity_input_schema(&activity, &input)?;
        Ok(ExecutionContext {
            activity,
            job: Some(job.clone()),
            agent_cli: job.agent_cli.clone(),
            timeout_seconds: job.timeout_seconds,
            input,
        })
    }

    fn execute_single_attempt(&self, execution: &ExecutionContext) -> AttemptOutcome {
        let agent = match Agent::new(
            &AgentConfig::cli(execution.agent_cli.clone()).with_codex_execution(
                self.context.codex_execution_policy.sandbox(),
                self.context.codex_execution_policy.approval_policy(),
            ),
        ) {
            Ok(agent) => agent,
            Err(err) => {
                return AttemptOutcome {
                    state: JobRunState::Failed,
                    exit_code: Some(1),
                    duration_ms: None,
                    response_json: None,
                    error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
                    error_message: Some(err.to_string()),
                    retryable: false,
                    protocol_violation: false,
                };
            }
        };
        let stdin_payload = match self.build_stdin_envelope_payload(execution) {
            Ok(payload) => payload,
            Err(err) => {
                return AttemptOutcome {
                    state: JobRunState::Failed,
                    exit_code: Some(1),
                    duration_ms: None,
                    response_json: None,
                    error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
                    error_message: Some(err.to_string()),
                    retryable: false,
                    protocol_violation: false,
                };
            }
        };
        let invocation_result = match &execution.job {
            Some(job) => agent.invoke(AgentRequest::job(
                job.job_id.clone(),
                execution.activity.id.clone(),
                stdin_payload,
            )),
            None => agent.invoke(AgentRequest::activity(
                execution.activity.id.clone(),
                stdin_payload,
            )),
        };
        let invocation = match invocation_result {
            Ok(invocation) => invocation,
            Err(err) => {
                return AttemptOutcome {
                    state: JobRunState::Failed,
                    exit_code: Some(1),
                    duration_ms: None,
                    response_json: None,
                    error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
                    error_message: Some(err.to_string()),
                    retryable: false,
                    protocol_violation: false,
                };
            }
        };
        let missing_env = self
            .context
            .execution_env_policy
            .missing_required(invocation.required_env_vars);
        if !missing_env.is_empty() {
            let vars = missing_env.join(", ");
            return AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: Some(1),
                duration_ms: None,
                response_json: None,
                error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
                error_message: Some(format!(
                    "missing required environment variable(s) for provider '{}': {vars}. \
configure .orbit/config.toml [execution.env].pass and set these variables in the parent shell.",
                    invocation.runtime_key
                )),
                retryable: false,
                protocol_violation: false,
            };
        };
        let environment_mode = if self.context.execution_env_policy.inherit() {
            EnvironmentMode::Inherit
        } else {
            EnvironmentMode::ClearAndSet(self.context.execution_env_policy.hydrated_allowlist_env())
        };
        let (args, _stdout_schema_file) = match prepare_exec_args(&invocation) {
            Ok(prepared) => prepared,
            Err(err) => {
                return AttemptOutcome {
                    state: JobRunState::Failed,
                    exit_code: Some(1),
                    duration_ms: None,
                    response_json: None,
                    error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
                    error_message: Some(err.to_string()),
                    retryable: true,
                    protocol_violation: false,
                };
            }
        };

        let exec_result = match run_process(
            &ExecRequest {
                program: invocation.program,
                args,
                timeout_ms: Some(execution.timeout_seconds.saturating_mul(1000)),
                stdin_mode: StdinMode::Bytes(invocation.stdin),
                environment_mode,
            },
            &NoSandbox,
        ) {
            Ok(result) => result,
            Err(err) => {
                return AttemptOutcome {
                    state: JobRunState::Failed,
                    exit_code: Some(1),
                    duration_ms: None,
                    response_json: None,
                    error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
                    error_message: Some(err.to_string()),
                    retryable: true,
                    protocol_violation: false,
                };
            }
        };

        if orbit_agent::is_timeout(&exec_result) && exec_result.stdout.trim().is_empty() {
            return AttemptOutcome {
                state: JobRunState::Timeout,
                exit_code: exec_result.exit_code,
                duration_ms: Some(exec_result.duration_ms),
                response_json: None,
                error_code: Some(AGENT_TIMEOUT.to_string()),
                error_message: Some(format_timeout_error_message(&exec_result)),
                retryable: true,
                protocol_violation: false,
            };
        }

        match parse_and_validate_response(&exec_result) {
            Ok((envelope, state)) => {
                let run_state = match state {
                    AgentResponseStatus::Success => JobRunState::Success,
                    AgentResponseStatus::Failed => JobRunState::Failed,
                    AgentResponseStatus::Timeout => JobRunState::Timeout,
                };
                let error_code = envelope.error.as_ref().map(|error| error.code.clone());
                let error_message = envelope.error.as_ref().map(|error| error.message.clone());
                if run_state == JobRunState::Success
                    && envelope.result.is_some()
                    && let Err(err) =
                        self.validate_skill_output_schema(&execution.activity, &envelope)
                {
                    return AttemptOutcome {
                        state: JobRunState::Failed,
                        exit_code: exec_result.exit_code,
                        duration_ms: Some(exec_result.duration_ms),
                        response_json: None,
                        error_code: Some(AGENT_PROTOCOL_VIOLATION.to_string()),
                        error_message: Some(err.to_string()),
                        retryable: false,
                        protocol_violation: true,
                    };
                }
                if run_state == JobRunState::Success
                    && let Some(result) = envelope.result.as_ref()
                    && let Err(err) = self.execute_commit_request_if_present(result)
                {
                    let (error_code, protocol_violation) = match err {
                        OrbitError::AgentProtocolViolation(_) => {
                            (AGENT_PROTOCOL_VIOLATION.to_string(), true)
                        }
                        _ => (AGENT_COMMIT_FAILED.to_string(), false),
                    };
                    return AttemptOutcome {
                        state: JobRunState::Failed,
                        exit_code: exec_result.exit_code,
                        duration_ms: Some(exec_result.duration_ms),
                        response_json: serde_json::to_value(envelope).ok(),
                        error_code: Some(error_code),
                        error_message: Some(err.to_string()),
                        retryable: false,
                        protocol_violation,
                    };
                }
                AttemptOutcome {
                    state: run_state,
                    exit_code: exec_result.exit_code,
                    duration_ms: Some(exec_result.duration_ms),
                    response_json: serde_json::to_value(envelope).ok(),
                    error_code,
                    error_message,
                    retryable: run_state == JobRunState::Failed
                        || run_state == JobRunState::Timeout,
                    protocol_violation: false,
                }
            }
            Err(OrbitError::AgentProtocolViolation(message)) => AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: exec_result.exit_code,
                duration_ms: Some(exec_result.duration_ms),
                response_json: None,
                error_code: Some(AGENT_PROTOCOL_VIOLATION.to_string()),
                error_message: Some(message),
                retryable: false,
                protocol_violation: true,
            },
            Err(err) => AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: exec_result.exit_code,
                duration_ms: Some(exec_result.duration_ms),
                response_json: None,
                error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
                error_message: Some(err.to_string()),
                retryable: true,
                protocol_violation: false,
            },
        }
    }

    fn build_stdin_envelope_payload(
        &self,
        execution: &ExecutionContext,
    ) -> Result<Vec<u8>, OrbitError> {
        let skills = self.resolve_activity_skill_refs(&execution.activity.skill_refs)?;
        let identity = execution
            .activity
            .identity_id
            .as_deref()
            .map(|identity_id| self.resolve_identity(identity_id))
            .transpose()?
            .map(|resolved| {
                json!({
                    "id": resolved.id,
                    "name": resolved.name,
                    "role": resolved.role.to_string(),
                    "block": self.compile_identity_block(&resolved),
                })
            });
        let envelope = ExecutionEnvelope {
            schema_version: 1,
            activity: json!({
                "id": execution.activity.id,
                "type": execution.activity.spec_type,
                "description": execution.activity.description,
                "instruction": execution.activity.instruction,
                "input_schema_json": execution.activity.input_schema_json,
                "output_schema_json": execution.activity.output_schema_json,
                "artifact_path_template": execution.activity.artifact_path_template,
                "skill_refs": execution.activity.skill_refs,
                "identity_id": execution.activity.identity_id,
                "assigned_to": execution.activity.assigned_to,
                "created_by": execution.activity.created_by,
            }),
            job: execution.job.as_ref().map(|job| {
                json!({
                    "id": job.job_id,
                    "target_type": job.target_type,
                    "target_id": job.target_id,
                    "schedule": job.schedule,
                    "agent_cli": job.agent_cli,
                    "timeout_seconds": job.timeout_seconds,
                    "retry_max_attempts": job.retry_max_attempts,
                    "retry_backoff_strategy": job.retry_backoff_strategy,
                    "retry_initial_delay_seconds": job.retry_initial_delay_seconds,
                    "state": job.state,
                    "next_run_at": job.next_run_at.to_rfc3339(),
                })
            }),
            skills: skills
                .into_iter()
                .map(|skill| ExecutionSkillEnvelope {
                    id: skill.id,
                    content_hash: skill.content_hash,
                    content: skill.content,
                    meta: skill.meta_raw,
                })
                .collect(),
            identity,
            input: execution.input.clone(),
            memory: json!({}),
        };

        serde_json::to_vec(&envelope)
            .map_err(|e| OrbitError::Execution(format!("failed to serialize stdin envelope: {e}")))
    }

    fn validate_skill_output_schema(
        &self,
        activity: &Activity,
        envelope: &AgentResponseEnvelope,
    ) -> Result<(), OrbitError> {
        let skills = self.resolve_activity_skill_refs(&activity.skill_refs)?;
        let Some(result) = envelope.result.as_ref() else {
            return Err(OrbitError::AgentProtocolViolation(
                "success response must include result payload".to_string(),
            ));
        };

        for skill in skills {
            let Some(schema) = skill.output_schema.as_ref() else {
                continue;
            };
            let context = format!("result does not match skill '{}' output schema", skill.id);
            if let Err(err) = validate_instance_against_schema(schema, result, &context) {
                return match err {
                    OrbitError::SkillValidation(message) => {
                        Err(OrbitError::AgentProtocolViolation(message))
                    }
                    other => Err(other),
                };
            }
        }

        Ok(())
    }

    fn execute_commit_request_if_present(&self, result: &Value) -> Result<(), OrbitError> {
        let Some(commit_value) = result.get("commit") else {
            return Ok(());
        };

        let commit: AgentCommitRequest =
            serde_json::from_value(commit_value.clone()).map_err(|error| {
                OrbitError::AgentProtocolViolation(format!(
                    "result.commit must be an object with string `message` and string-array `files`: {error}"
                ))
            })?;

        if commit.message.trim().is_empty() {
            return Err(OrbitError::AgentProtocolViolation(
                "result.commit.message must not be empty".to_string(),
            ));
        }
        if commit.files.is_empty() {
            return Err(OrbitError::AgentProtocolViolation(
                "result.commit.files must contain at least one path".to_string(),
            ));
        }
        let files = commit.files.clone();
        let message = commit.message.clone();

        let repo_root = paths::find_git_repo_root(&self.context.data_root).ok_or_else(|| {
            OrbitError::Execution(format!(
                "cannot locate git repository root from Orbit data root '{}'",
                self.context.data_root.display()
            ))
        })?;
        let repo_root_str = repo_root.to_string_lossy().to_string();

        self.run_tool(
            "git.stage_paths",
            json!({
                "repo_root": repo_root_str,
                "files": files.clone(),
            }),
        )?;
        self.run_tool(
            "git.commit",
            json!({
                "repo_root": repo_root.to_string_lossy(),
                "message": message,
                "files": files,
            }),
        )?;
        Ok(())
    }

    fn validate_activity_input_schema(
        &self,
        activity: &Activity,
        input: &Value,
    ) -> Result<(), OrbitError> {
        let context = format!(
            "job run input does not match activity '{}' input schema",
            activity.id
        );
        match validate_instance_against_schema(&activity.input_schema_json, input, &context) {
            Ok(()) => Ok(()),
            Err(OrbitError::AgentProtocolViolation(message)) => {
                Err(OrbitError::InvalidInput(message))
            }
            Err(other) => Err(other),
        }
    }

    fn validate_activity_target_exists(
        &self,
        target_type: JobTargetType,
        target_id: &str,
    ) -> Result<(), OrbitError> {
        let _ = target_type;
        let activity = self.show_activity(target_id)?;
        let _ = self.resolve_activity_skill_refs(&activity.skill_refs)?;
        Ok(())
    }

    fn list_jobs_backend(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        self.context.job_store.list_jobs(include_disabled)
    }

    fn get_job_backend(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.context.job_store.get_job(job_id)
    }

    fn get_pending_or_running_job_run_backend(
        &self,
        job_id: &str,
    ) -> Result<Option<JobRun>, OrbitError> {
        self.context
            .job_store
            .get_pending_or_running_job_run(job_id)
    }

    fn insert_job_run_backend(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<JobRun, OrbitError> {
        self.context
            .job_store
            .insert_job_run(job_id, attempt, scheduled_at)
    }

    fn mark_job_run_running_backend(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.context
            .job_store
            .mark_job_run_running(run_id, started_at)
    }

    fn complete_job_run_backend(
        &self,
        params: &JobRunCompletionParams,
    ) -> Result<bool, OrbitError> {
        self.context.job_store.complete_job_run(params)
    }

    fn update_job_next_run_backend(
        &self,
        job_id: &str,
        next_run_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.context
            .job_store
            .update_job_next_run(job_id, next_run_at)
    }
}

fn is_stale_active_run(job: &Job, run: &JobRun, now: DateTime<Utc>) -> bool {
    let reference_time = run.started_at.unwrap_or(run.created_at);
    let elapsed_seconds = now.signed_duration_since(reference_time).num_seconds();
    let stale_after_seconds = job.timeout_seconds.saturating_add(STALE_RUN_GRACE_SECONDS) as i64;
    elapsed_seconds >= stale_after_seconds
}

fn prepare_exec_args(
    invocation: &orbit_agent::AgentResponse,
) -> Result<(Vec<String>, Option<NamedTempFile>), OrbitError> {
    let mut args = invocation.args.clone();
    let mut stdout_schema_file = None;

    if let Some(schema) = invocation.stdout_schema_json.as_ref() {
        let mut file = NamedTempFile::new().map_err(|error| {
            OrbitError::Execution(format!(
                "failed to create temporary agent output schema file: {error}"
            ))
        })?;
        serde_json::to_writer(file.as_file_mut(), schema).map_err(|error| {
            OrbitError::Execution(format!(
                "failed to write temporary agent output schema file: {error}"
            ))
        })?;
        file.as_file_mut().flush().map_err(|error| {
            OrbitError::Execution(format!(
                "failed to flush temporary agent output schema file: {error}"
            ))
        })?;

        args.push("--output-schema".to_string());
        args.push(file.path().to_string_lossy().into_owned());
        stdout_schema_file = Some(file);
    }

    Ok((args, stdout_schema_file))
}

fn format_timeout_error_message(exec_result: &orbit_types::ExecutionResult) -> String {
    let stderr = exec_result.stderr.trim();
    if stderr.is_empty() {
        return "agent timed out before producing JSON stdout".to_string();
    }
    format!("agent timed out before producing JSON stdout; stderr: {stderr}")
}

fn manual_job_next_run_at() -> DateTime<Utc> {
    Utc::now() + chrono::Duration::days(365 * 100)
}

const DEFAULT_NAMED_JOBS: &[(&str, &str)] = &[
    ("job-resolve-backlogged-task", "resolve-backlogged-task"),
    ("job-perform-maintenance", "perform-maintenance"),
    ("job-oversee-orbit-operations", "oversee-orbit-operations"),
    ("job-approve-task-leader", "approve-task-leader"),
    ("job-triage-and-dispatch-task", "triage-and-dispatch-task"),
];

pub(crate) fn seed_default_jobs(runtime: &OrbitRuntime) -> Result<usize, OrbitError> {
    let mut created = 0usize;
    for (job_id, target_id) in DEFAULT_NAMED_JOBS {
        if runtime.show_job(job_id).is_ok() {
            continue;
        }
        runtime.add_job(JobAddParams {
            job_id: Some(job_id.to_string()),
            target_type: JobTargetType::Activity,
            target_id: target_id.to_string(),
            schedule: "manual".to_string(),
            agent_cli: "codex".to_string(), // TODO: make this dynamic
            timeout_seconds: 900,
            retry_max_attempts: 0,
            retry_backoff_strategy: JobRetryBackoffStrategy::None,
            retry_initial_delay_seconds: 0,
            initial_state_override: Some(JobScheduleState::Enabled),
        })?;
        created += 1;
    }
    Ok(created)
}
