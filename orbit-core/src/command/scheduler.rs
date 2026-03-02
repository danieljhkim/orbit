use chrono::{DateTime, Utc};
use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_store::ClaimedJobRun;
use orbit_store::SchedulerCreateParams as StoreJobCreateParams;
use orbit_types::{
    AgentResponseEnvelope, OrbitError, OrbitEvent, Scheduler, SchedulerRetryBackoffStrategy,
    SchedulerRun, SchedulerRunState, SchedulerScheduleState, SchedulerTargetType,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::json_schema::validate_instance_against_schema;
const AGENT_PROTOCOL_VIOLATION: &str = "AGENT_PROTOCOL_VIOLATION";
const AGENT_INVOCATION_FAILED: &str = "AGENT_INVOCATION_FAILED";
const STALE_RUN_GRACE_SECONDS: u64 = 30;

#[derive(Debug, Clone)]
pub struct SchedulerAddParams {
    pub target_type: SchedulerTargetType,
    pub target_id: String,
    pub schedule: String,
    pub agent_cli: String,
    pub timeout_seconds: u64,
    pub retry_max_attempts: u32,
    pub retry_backoff_strategy: SchedulerRetryBackoffStrategy,
    pub retry_initial_delay_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct SchedulerRunResult {
    pub scheduler_id: String,
    pub run_id: String,
    pub state: SchedulerRunState,
    pub attempt: u32,
}

#[derive(Debug, Clone)]
struct AttemptOutcome {
    state: SchedulerRunState,
    exit_code: Option<i32>,
    duration_ms: Option<u64>,
    response_json: Option<Value>,
    error_code: Option<String>,
    error_message: Option<String>,
    retryable: bool,
    protocol_violation: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ScheduledExecutionEnvelope {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    job: Value,
    skills: Vec<ScheduledSkillEnvelope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identity: Option<Value>,
    input: Value,
    memory: Value,
}

#[derive(Debug, Clone, Serialize)]
struct ScheduledSkillEnvelope {
    id: String,
    content_hash: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<Value>,
}

impl OrbitRuntime {
    pub fn add_scheduler(&self, params: SchedulerAddParams) -> Result<Scheduler, OrbitError> {
        if params.target_id.trim().is_empty() {
            return Err(OrbitError::SchedulerValidation(
                "target_id must not be empty".to_string(),
            ));
        }
        if params.schedule.trim().is_empty() {
            return Err(OrbitError::SchedulerValidation(
                "schedule must not be empty".to_string(),
            ));
        }
        if params.agent_cli.trim().is_empty() {
            return Err(OrbitError::SchedulerValidation(
                "agent_cli must not be empty".to_string(),
            ));
        }

        self.validate_job_target_exists(params.target_type, &params.target_id)?;

        // Validate provider adapter availability at add-time.
        let _ = crate::scheduler::agent_protocol::build_invocation(
            &params.agent_cli,
            params.target_type,
            &params.target_id,
        )?;

        let next_run_at =
            crate::scheduler::state_machine::compute_next_run_at(&params.schedule, Utc::now())?;

        let scheduler = self
            .context
            .scheduler_store
            .add_scheduler(StoreJobCreateParams {
                target_type: params.target_type,
                target_id: params.target_id,
                schedule: params.schedule,
                agent_cli: params.agent_cli,
                timeout_seconds: params.timeout_seconds,
                retry_max_attempts: params.retry_max_attempts,
                retry_backoff_strategy: params.retry_backoff_strategy,
                retry_initial_delay_seconds: params.retry_initial_delay_seconds,
                next_run_at,
            })?;
        self.record_event(OrbitEvent::SchedulerAdded {
            scheduler_id: scheduler.scheduler_id.clone(),
        })?;
        Ok(scheduler)
    }

    pub fn list_schedulers(&self, include_disabled: bool) -> Result<Vec<Scheduler>, OrbitError> {
        self.list_jobs_backend(include_disabled)
    }

    pub fn show_scheduler(&self, scheduler_id: &str) -> Result<Scheduler, OrbitError> {
        self.get_job_backend(scheduler_id)?
            .ok_or_else(|| OrbitError::SchedulerNotFound(scheduler_id.to_string()))
    }

    pub fn pause_scheduler(&self, scheduler_id: &str) -> Result<(), OrbitError> {
        let _ = self.show_scheduler(scheduler_id)?;
        let changed = self
            .context
            .scheduler_store
            .set_scheduler_state(scheduler_id, SchedulerScheduleState::Paused)?;
        if !changed {
            return Err(OrbitError::SchedulerNotFound(scheduler_id.to_string()));
        }
        self.record_event(OrbitEvent::SchedulerPaused {
            scheduler_id: scheduler_id.to_string(),
        })
    }

    pub fn resume_scheduler(&self, scheduler_id: &str) -> Result<(), OrbitError> {
        let scheduler = self.show_scheduler(scheduler_id)?;
        let next_run_at =
            crate::scheduler::state_machine::compute_next_run_at(&scheduler.schedule, Utc::now())?;

        let changed = self
            .context
            .scheduler_store
            .set_scheduler_state(scheduler_id, SchedulerScheduleState::Enabled)?;
        if !changed {
            return Err(OrbitError::SchedulerNotFound(scheduler_id.to_string()));
        }
        let _ = self
            .context
            .scheduler_store
            .update_scheduler_next_run(scheduler_id, next_run_at)?;
        self.record_event(OrbitEvent::SchedulerResumed {
            scheduler_id: scheduler_id.to_string(),
        })
    }

    pub fn delete_scheduler(&self, scheduler_id: &str) -> Result<(), OrbitError> {
        let changed = self
            .context
            .scheduler_store
            .mark_scheduler_disabled(scheduler_id)?;
        if !changed {
            return Err(OrbitError::SchedulerNotFound(scheduler_id.to_string()));
        }
        self.record_event(OrbitEvent::SchedulerDeleted {
            scheduler_id: scheduler_id.to_string(),
        })
    }

    pub fn scheduler_history(&self, scheduler_id: &str) -> Result<Vec<SchedulerRun>, OrbitError> {
        let scheduler = self.show_scheduler(scheduler_id)?;
        let _ = self.recover_stale_active_run_for_job(&scheduler, Utc::now())?;
        self.list_job_runs_backend(scheduler_id)
    }

    pub fn run_scheduler_now(&self, scheduler_id: &str) -> Result<SchedulerRunResult, OrbitError> {
        let scheduler = self.show_scheduler(scheduler_id)?;
        let _ = self.recover_stale_active_run_for_job(&scheduler, Utc::now())?;
        if let Some(active_run) = self.get_pending_or_running_job_run_backend(scheduler_id)? {
            return Err(OrbitError::SchedulerValidation(format!(
                "scheduler '{}' already has an active run '{}' in state '{}'",
                scheduler_id, active_run.run_id, active_run.state
            )));
        }
        self.record_event(OrbitEvent::SchedulerTriggered {
            scheduler_id: scheduler.scheduler_id.clone(),
        })?;

        self.execute_job_with_retries(scheduler, Utc::now(), None)
    }

    pub(crate) fn execute_claimed_job(&self, claimed: &ClaimedJobRun) -> Result<(), OrbitError> {
        let _ = self.execute_job_with_retries(
            claimed.scheduler.clone(),
            claimed.run.scheduled_at,
            Some(claimed.run.clone()),
        )?;
        Ok(())
    }

    fn execute_job_with_retries(
        &self,
        scheduler: Scheduler,
        scheduled_at: DateTime<Utc>,
        initial_run: Option<SchedulerRun>,
    ) -> Result<SchedulerRunResult, OrbitError> {
        let max_attempts = scheduler.retry_max_attempts.saturating_add(1);
        let mut current_attempt = initial_run.as_ref().map(|r| r.attempt).unwrap_or(1);
        let mut pending_initial = initial_run;
        let mut last_result: Option<SchedulerRunResult> = None;
        let mut retry_scheduled_for_future = false;

        while current_attempt <= max_attempts {
            let mut run = if let Some(existing) = pending_initial.take() {
                existing
            } else {
                let run = self.insert_job_run_backend(
                    &scheduler.scheduler_id,
                    current_attempt,
                    scheduled_at,
                )?;
                self.record_event(OrbitEvent::SchedulerRunStarted {
                    scheduler_id: scheduler.scheduler_id.clone(),
                    run_id: String::new(),
                    attempt: current_attempt,
                })?;
                run
            };

            let started_at = Utc::now();
            let changed = self.mark_job_run_running_backend(&run.run_id, started_at)?;
            if !changed {
                return Err(OrbitError::SchedulerRunNotFound(run.run_id.clone()));
            }
            self.record_event(OrbitEvent::SchedulerRunStarted {
                scheduler_id: scheduler.scheduler_id.clone(),
                run_id: run.run_id.clone(),
                attempt: run.attempt,
            })?;
            run.state = SchedulerRunState::Running;
            run.started_at = Some(started_at);

            let outcome = self.execute_single_attempt(&scheduler);
            let finished_at = Utc::now();

            let changed = self.complete_scheduler_run_backend(
                &run.run_id,
                outcome.state,
                finished_at,
                outcome.duration_ms,
                outcome.exit_code,
                outcome.response_json.as_ref(),
                outcome.error_code.as_deref(),
                outcome.error_message.as_deref(),
            )?;
            if !changed {
                return Err(OrbitError::SchedulerRunNotFound(run.run_id.clone()));
            }
            self.record_event(OrbitEvent::SchedulerRunCompleted {
                scheduler_id: scheduler.scheduler_id.clone(),
                run_id: run.run_id.clone(),
                state: outcome.state.to_string(),
            })?;

            if outcome.protocol_violation {
                self.record_event(OrbitEvent::SchedulerProtocolViolation {
                    scheduler_id: scheduler.scheduler_id.clone(),
                    run_id: run.run_id.clone(),
                    message: outcome
                        .error_message
                        .clone()
                        .unwrap_or_else(|| "agent protocol violation".to_string()),
                })?;
            }

            last_result = Some(SchedulerRunResult {
                scheduler_id: scheduler.scheduler_id.clone(),
                run_id: run.run_id.clone(),
                state: outcome.state,
                attempt: run.attempt,
            });

            if outcome.state == SchedulerRunState::Success {
                break;
            }

            if outcome.retryable && current_attempt < max_attempts {
                let retry_index = current_attempt;
                let delay_seconds = crate::scheduler::state_machine::compute_retry_delay_seconds(
                    scheduler.retry_backoff_strategy,
                    scheduler.retry_initial_delay_seconds,
                    retry_index,
                );
                let next_retry_at = Utc::now() + chrono::Duration::seconds(delay_seconds as i64);

                let _ = self.update_job_next_run_backend(&scheduler.scheduler_id, next_retry_at)?;
                self.record_event(OrbitEvent::SchedulerRetryScheduled {
                    scheduler_id: scheduler.scheduler_id.clone(),
                    run_id: run.run_id.clone(),
                    next_run_at: next_retry_at.to_rfc3339(),
                })?;

                if delay_seconds > 0 {
                    // Avoid blocking scheduler execution while waiting for delayed retries.
                    retry_scheduled_for_future = true;
                    break;
                }

                current_attempt = current_attempt.saturating_add(1);
                continue;
            }

            break;
        }

        if !retry_scheduled_for_future {
            let next_run_at = crate::scheduler::state_machine::compute_next_run_at(
                &scheduler.schedule,
                Utc::now(),
            )?;
            let _ = self.update_job_next_run_backend(&scheduler.scheduler_id, next_run_at);
            let _ = self.record_event(OrbitEvent::SchedulerTriggered {
                scheduler_id: scheduler.scheduler_id.clone(),
            });
        }

        last_result.ok_or(OrbitError::SchedulerRunNotFound(scheduler.scheduler_id))
    }

    pub(crate) fn recover_stale_active_run_for_job(
        &self,
        scheduler: &Scheduler,
        now: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        let Some(active_run) =
            self.get_pending_or_running_job_run_backend(&scheduler.scheduler_id)?
        else {
            return Ok(false);
        };

        if !is_stale_active_run(scheduler, &active_run, now) {
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
            scheduler.timeout_seconds,
            STALE_RUN_GRACE_SECONDS
        );

        let changed = self.complete_scheduler_run_backend(
            &active_run.run_id,
            SchedulerRunState::Failed,
            now,
            duration_ms,
            Some(1),
            None,
            Some(AGENT_INVOCATION_FAILED),
            Some(&message),
        )?;
        if !changed {
            return Err(OrbitError::SchedulerRunNotFound(active_run.run_id.clone()));
        }
        self.record_event(OrbitEvent::SchedulerRunCompleted {
            scheduler_id: scheduler.scheduler_id.clone(),
            run_id: active_run.run_id.clone(),
            state: SchedulerRunState::Failed.to_string(),
        })?;

        Ok(true)
    }

    fn execute_single_attempt(&self, scheduler: &Scheduler) -> AttemptOutcome {
        let invocation = match crate::scheduler::agent_protocol::build_invocation(
            &scheduler.agent_cli,
            scheduler.target_type,
            &scheduler.target_id,
        ) {
            Ok(invocation) => invocation,
            Err(err) => {
                return AttemptOutcome {
                    state: SchedulerRunState::Failed,
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
        let provider = crate::scheduler::agent_protocol::provider_key(&scheduler.agent_cli);
        let missing_env = self
            .context
            .execution_env_policy
            .missing_required_for_provider(&provider);
        if !missing_env.is_empty() {
            let vars = missing_env.join(", ");
            return AttemptOutcome {
                state: SchedulerRunState::Failed,
                exit_code: Some(1),
                duration_ms: None,
                response_json: None,
                error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
                error_message: Some(format!(
                    "missing required environment variable(s) for provider '{provider}': {vars}. \
configure .orbit/config.toml [execution.env].pass and set these variables in the parent shell."
                )),
                retryable: false,
                protocol_violation: false,
            };
        }
        let environment_mode = if self.context.execution_env_policy.inherit() {
            EnvironmentMode::Inherit
        } else {
            EnvironmentMode::ClearAndSet(self.context.execution_env_policy.hydrated_allowlist_env())
        };
        let stdin_payload = match self.build_stdin_envelope_payload(scheduler) {
            Ok(payload) => payload,
            Err(err) => {
                return AttemptOutcome {
                    state: SchedulerRunState::Failed,
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
        let stdin_payload =
            crate::scheduler::agent_protocol::build_stdin_payload(&invocation, &stdin_payload);

        let exec_result = match run_process(
            &ExecRequest {
                program: invocation.program,
                args: invocation.args,
                timeout_ms: Some(scheduler.timeout_seconds.saturating_mul(1000)),
                stdin_mode: StdinMode::Bytes(stdin_payload),
                environment_mode,
            },
            &NoSandbox,
        ) {
            Ok(result) => result,
            Err(err) => {
                return AttemptOutcome {
                    state: SchedulerRunState::Failed,
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

        match crate::scheduler::agent_protocol::parse_and_validate_response(&exec_result) {
            Ok((envelope, state)) => {
                if state == SchedulerRunState::Success
                    && let Err(err) = self.validate_skill_output_schema(scheduler, &envelope)
                {
                    return AttemptOutcome {
                        state: SchedulerRunState::Failed,
                        exit_code: exec_result.exit_code,
                        duration_ms: Some(exec_result.duration_ms),
                        response_json: None,
                        error_code: Some(AGENT_PROTOCOL_VIOLATION.to_string()),
                        error_message: Some(err.to_string()),
                        retryable: false,
                        protocol_violation: true,
                    };
                }
                AttemptOutcome {
                    state,
                    exit_code: exec_result.exit_code,
                    duration_ms: Some(exec_result.duration_ms),
                    response_json: serde_json::to_value(envelope).ok(),
                    error_code: None,
                    error_message: None,
                    retryable: state == SchedulerRunState::Failed
                        || state == SchedulerRunState::Timeout,
                    protocol_violation: false,
                }
            }
            Err(OrbitError::AgentProtocolViolation(message)) => AttemptOutcome {
                state: SchedulerRunState::Failed,
                exit_code: exec_result.exit_code,
                duration_ms: Some(exec_result.duration_ms),
                response_json: None,
                error_code: Some(AGENT_PROTOCOL_VIOLATION.to_string()),
                error_message: Some(message),
                retryable: false,
                protocol_violation: true,
            },
            Err(err) => AttemptOutcome {
                state: SchedulerRunState::Failed,
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

    fn build_stdin_envelope_payload(&self, scheduler: &Scheduler) -> Result<Vec<u8>, OrbitError> {
        let envelope = {
            let job = self.show_job(&scheduler.target_id)?;
            let skills = self.resolve_job_skill_refs(&job.skill_refs)?;
            let identity = job
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
            ScheduledExecutionEnvelope {
                schema_version: 1,
                job: json!({
                    "id": job.id,
                    "type": job.spec_type,
                    "description": job.description,
                    "input_schema_json": job.input_schema_json,
                    "output_schema_json": job.output_schema_json,
                    "artifact_path_template": job.artifact_path_template,
                    "skill_refs": job.skill_refs,
                    "identity_id": job.identity_id,
                    "assigned_to": job.assigned_to,
                    "created_by": job.created_by,
                }),
                skills: skills
                    .into_iter()
                    .map(|skill| ScheduledSkillEnvelope {
                        id: skill.id,
                        content_hash: skill.content_hash,
                        content: skill.content,
                        meta: skill.meta_raw,
                    })
                    .collect(),
                identity,
                input: json!({}),
                memory: json!({}),
            }
        };

        serde_json::to_vec(&envelope)
            .map_err(|e| OrbitError::Execution(format!("failed to serialize stdin envelope: {e}")))
    }

    fn validate_skill_output_schema(
        &self,
        scheduler: &Scheduler,
        envelope: &AgentResponseEnvelope,
    ) -> Result<(), OrbitError> {
        if scheduler.target_type != SchedulerTargetType::Job {
            return Ok(());
        }
        let job = self.show_job(&scheduler.target_id)?;
        let skills = self.resolve_job_skill_refs(&job.skill_refs)?;
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

    fn validate_job_target_exists(
        &self,
        target_type: SchedulerTargetType,
        target_id: &str,
    ) -> Result<(), OrbitError> {
        let _ = target_type;
        let job = self.show_job(target_id)?;
        let _ = self.resolve_job_skill_refs(&job.skill_refs)?;
        Ok(())
    }

    fn list_jobs_backend(&self, include_disabled: bool) -> Result<Vec<Scheduler>, OrbitError> {
        self.context
            .scheduler_store
            .list_schedulers(include_disabled)
    }

    fn get_job_backend(&self, scheduler_id: &str) -> Result<Option<Scheduler>, OrbitError> {
        self.context.scheduler_store.get_scheduler(scheduler_id)
    }

    fn list_job_runs_backend(&self, scheduler_id: &str) -> Result<Vec<SchedulerRun>, OrbitError> {
        self.context
            .scheduler_store
            .list_scheduler_runs(scheduler_id)
    }

    fn get_pending_or_running_job_run_backend(
        &self,
        scheduler_id: &str,
    ) -> Result<Option<SchedulerRun>, OrbitError> {
        self.context
            .scheduler_store
            .get_pending_or_running_scheduler_run(scheduler_id)
    }

    fn insert_job_run_backend(
        &self,
        scheduler_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<SchedulerRun, OrbitError> {
        self.context
            .scheduler_store
            .insert_scheduler_run(scheduler_id, attempt, scheduled_at)
    }

    fn mark_job_run_running_backend(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.context
            .scheduler_store
            .mark_scheduler_run_running(run_id, started_at)
    }

    #[allow(clippy::too_many_arguments)]
    fn complete_scheduler_run_backend(
        &self,
        run_id: &str,
        state: SchedulerRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
        exit_code: Option<i32>,
        agent_response_json: Option<&Value>,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<bool, OrbitError> {
        self.context.scheduler_store.complete_scheduler_run(
            run_id,
            state,
            finished_at,
            duration_ms,
            exit_code,
            agent_response_json,
            error_code,
            error_message,
        )
    }

    fn update_job_next_run_backend(
        &self,
        scheduler_id: &str,
        next_run_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.context
            .scheduler_store
            .update_scheduler_next_run(scheduler_id, next_run_at)
    }
}

fn is_stale_active_run(scheduler: &Scheduler, run: &SchedulerRun, now: DateTime<Utc>) -> bool {
    let reference_time = run.started_at.unwrap_or(run.created_at);
    let elapsed_seconds = now.signed_duration_since(reference_time).num_seconds();
    let stale_after_seconds = scheduler
        .timeout_seconds
        .saturating_add(STALE_RUN_GRACE_SECONDS) as i64;
    elapsed_seconds >= stale_after_seconds
}
