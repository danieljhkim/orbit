use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use orbit_exec::{ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_store::ClaimedJobRun;
use orbit_types::{
    AgentResponseEnvelope, Job, JobRetryBackoffStrategy, JobRun, JobRunState, JobScheduleState,
    JobTargetType, OrbitError, OrbitEvent,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::json_schema::validate_instance_against_schema;
const AGENT_PROTOCOL_VIOLATION: &str = "AGENT_PROTOCOL_VIOLATION";
const AGENT_INVOCATION_FAILED: &str = "AGENT_INVOCATION_FAILED";

#[derive(Debug, Clone)]
pub struct JobAddParams {
    pub target_type: JobTargetType,
    pub target_id: String,
    pub schedule: String,
    pub agent_cli: String,
    pub timeout_seconds: u64,
    pub retry_max_attempts: u32,
    pub retry_backoff_strategy: JobRetryBackoffStrategy,
    pub retry_initial_delay_seconds: u64,
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
struct ScheduledExecutionEnvelope {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    work: Value,
    skills: Vec<ScheduledSkillEnvelope>,
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

        self.validate_job_target_exists(params.target_type, &params.target_id)?;

        // Validate provider adapter availability at add-time.
        let _ = crate::job::agent_protocol::build_invocation(
            &params.agent_cli,
            params.target_type,
            &params.target_id,
        )?;

        let next_run_at =
            crate::job::state_machine::compute_next_run_at(&params.schedule, Utc::now())?;

        self.with_mutation(|tx| {
            let job = tx.insert_job_v2(
                params.target_type,
                &params.target_id,
                &params.schedule,
                &params.agent_cli,
                params.timeout_seconds,
                params.retry_max_attempts,
                params.retry_backoff_strategy,
                params.retry_initial_delay_seconds,
                next_run_at,
            )?;
            Ok((
                job.clone(),
                OrbitEvent::JobAdded {
                    job_id: job.job_id.clone(),
                },
            ))
        })
    }

    pub fn list_jobs(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        self.context.store.list_jobs(include_disabled)
    }

    pub fn show_job(&self, job_id: &str) -> Result<Job, OrbitError> {
        self.context
            .store
            .get_job(job_id)?
            .ok_or_else(|| OrbitError::JobNotFound(job_id.to_string()))
    }

    pub fn pause_job(&self, job_id: &str) -> Result<(), OrbitError> {
        let _ = self.show_job(job_id)?;
        self.with_mutation(|tx| {
            let changed = tx.set_job_state(job_id, JobScheduleState::Paused)?;
            if !changed {
                return Err(OrbitError::JobNotFound(job_id.to_string()));
            }
            Ok((
                (),
                OrbitEvent::JobPaused {
                    job_id: job_id.to_string(),
                },
            ))
        })
    }

    pub fn resume_job(&self, job_id: &str) -> Result<(), OrbitError> {
        let job = self.show_job(job_id)?;
        let next_run_at =
            crate::job::state_machine::compute_next_run_at(&job.schedule, Utc::now())?;

        self.with_mutation(|tx| {
            let changed = tx.set_job_state(job_id, JobScheduleState::Enabled)?;
            if !changed {
                return Err(OrbitError::JobNotFound(job_id.to_string()));
            }
            let _ = tx.update_job_next_run(job_id, next_run_at)?;
            Ok((
                (),
                OrbitEvent::JobResumed {
                    job_id: job_id.to_string(),
                },
            ))
        })
    }

    pub fn delete_job(&self, job_id: &str) -> Result<(), OrbitError> {
        self.with_mutation(|tx| {
            let changed = tx.mark_job_disabled(job_id)?;
            if !changed {
                return Err(OrbitError::JobNotFound(job_id.to_string()));
            }
            Ok((
                (),
                OrbitEvent::JobDeleted {
                    job_id: job_id.to_string(),
                },
            ))
        })
    }

    pub fn job_history(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        let _ = self.show_job(job_id)?;
        self.context.store.list_job_runs(job_id)
    }

    pub fn run_job_now(&self, job_id: &str) -> Result<JobRunResult, OrbitError> {
        let job = self.show_job(job_id)?;
        self.with_mutation(|_| {
            Ok((
                (),
                OrbitEvent::JobTriggered {
                    job_id: job.job_id.clone(),
                },
            ))
        })?;

        self.execute_job_with_retries(job, Utc::now(), None)
    }

    pub(crate) fn execute_claimed_job(&self, claimed: &ClaimedJobRun) -> Result<(), OrbitError> {
        let _ = self.execute_job_with_retries(
            claimed.job.clone(),
            claimed.run.scheduled_at,
            Some(claimed.run.clone()),
        )?;
        Ok(())
    }

    fn execute_job_with_retries(
        &self,
        job: Job,
        scheduled_at: DateTime<Utc>,
        initial_run: Option<JobRun>,
    ) -> Result<JobRunResult, OrbitError> {
        let max_attempts = job.retry_max_attempts.saturating_add(1);
        let mut current_attempt = initial_run.as_ref().map(|r| r.attempt).unwrap_or(1);
        let mut pending_initial = initial_run;
        let mut last_result: Option<JobRunResult> = None;

        while current_attempt <= max_attempts {
            let mut run = if let Some(existing) = pending_initial.take() {
                existing
            } else {
                self.with_mutation(|tx| {
                    let run = tx.insert_job_run(&job.job_id, current_attempt, scheduled_at)?;
                    Ok((
                        run,
                        OrbitEvent::JobRunStarted {
                            job_id: job.job_id.clone(),
                            run_id: String::new(),
                            attempt: current_attempt,
                        },
                    ))
                })?
            };

            let started_at = Utc::now();
            self.with_mutation(|tx| {
                let changed = tx.mark_job_run_running(&run.run_id, started_at)?;
                if !changed {
                    return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
                }
                Ok((
                    (),
                    OrbitEvent::JobRunStarted {
                        job_id: job.job_id.clone(),
                        run_id: run.run_id.clone(),
                        attempt: run.attempt,
                    },
                ))
            })?;
            run.state = JobRunState::Running;
            run.started_at = Some(started_at);

            let outcome = self.execute_single_attempt(&job);
            let finished_at = Utc::now();

            self.with_mutation(|tx| {
                let changed = tx.complete_job_run(
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
                    return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
                }

                Ok((
                    (),
                    OrbitEvent::JobRunCompleted {
                        job_id: job.job_id.clone(),
                        run_id: run.run_id.clone(),
                        state: outcome.state.to_string(),
                    },
                ))
            })?;

            if outcome.protocol_violation {
                self.with_mutation(|_| {
                    Ok((
                        (),
                        OrbitEvent::JobProtocolViolation {
                            job_id: job.job_id.clone(),
                            run_id: run.run_id.clone(),
                            message: outcome
                                .error_message
                                .clone()
                                .unwrap_or_else(|| "agent protocol violation".to_string()),
                        },
                    ))
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

                self.with_mutation(|tx| {
                    let _ = tx.update_job_next_run(&job.job_id, next_retry_at)?;
                    Ok((
                        (),
                        OrbitEvent::JobRetryScheduled {
                            job_id: job.job_id.clone(),
                            run_id: run.run_id.clone(),
                            next_run_at: next_retry_at.to_rfc3339(),
                        },
                    ))
                })?;

                if delay_seconds > 0 {
                    thread::sleep(Duration::from_secs(delay_seconds));
                }

                current_attempt = current_attempt.saturating_add(1);
                continue;
            }

            break;
        }

        let next_run_at =
            crate::job::state_machine::compute_next_run_at(&job.schedule, Utc::now())?;
        let _ = self.with_mutation(|tx| {
            let _ = tx.update_job_next_run(&job.job_id, next_run_at)?;
            Ok((
                (),
                OrbitEvent::JobTriggered {
                    job_id: job.job_id.clone(),
                },
            ))
        });

        last_result.ok_or(OrbitError::JobRunNotFound(job.job_id))
    }

    fn execute_single_attempt(&self, job: &Job) -> AttemptOutcome {
        let invocation = match crate::job::agent_protocol::build_invocation(
            &job.agent_cli,
            job.target_type,
            &job.target_id,
        ) {
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
        let stdin_payload = match self.build_stdin_envelope_payload(job) {
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
        let stdin_payload =
            crate::job::agent_protocol::build_stdin_payload(&invocation, &stdin_payload);

        let exec_result = match run_process(
            &ExecRequest {
                program: invocation.program,
                args: invocation.args,
                timeout_ms: Some(job.timeout_seconds.saturating_mul(1000)),
                stdin_mode: StdinMode::Bytes(stdin_payload),
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

        match crate::job::agent_protocol::parse_and_validate_response(&exec_result) {
            Ok((envelope, state)) => {
                if state == JobRunState::Success
                    && let Err(err) = self.validate_skill_output_schema(job, &envelope)
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
                AttemptOutcome {
                    state,
                    exit_code: exec_result.exit_code,
                    duration_ms: Some(exec_result.duration_ms),
                    response_json: serde_json::to_value(envelope).ok(),
                    error_code: None,
                    error_message: None,
                    retryable: state == JobRunState::Failed || state == JobRunState::Timeout,
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

    fn build_stdin_envelope_payload(&self, job: &Job) -> Result<Vec<u8>, OrbitError> {
        let envelope = match job.target_type {
            JobTargetType::Work => {
                let work = self
                    .context
                    .store
                    .get_work(&job.target_id)?
                    .ok_or_else(|| OrbitError::WorkNotFound(job.target_id.clone()))?;
                let skills = self.resolve_work_skill_refs(&work.skill_refs)?;
                ScheduledExecutionEnvelope {
                    schema_version: 1,
                    work: json!({
                        "id": work.id,
                        "type": work.spec_type,
                        "description": work.description,
                        "input_schema_json": work.input_schema_json,
                        "output_schema_json": work.output_schema_json,
                        "artifact_path_template": work.artifact_path_template,
                        "skill_refs": work.skill_refs,
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
                    input: json!({}),
                    memory: json!({}),
                }
            }
            JobTargetType::Workflow => {
                let workflow = self
                    .context
                    .store
                    .get_workflow(&job.target_id)?
                    .ok_or_else(|| OrbitError::WorkflowNotFound(job.target_id.clone()))?;
                ScheduledExecutionEnvelope {
                    schema_version: 1,
                    work: json!({
                        "id": workflow.id,
                        "type": "workflow",
                        "name": workflow.name,
                        "definition_json": workflow.definition_json,
                    }),
                    skills: Vec::new(),
                    input: json!({}),
                    memory: json!({}),
                }
            }
        };

        serde_json::to_vec(&envelope)
            .map_err(|e| OrbitError::Execution(format!("failed to serialize stdin envelope: {e}")))
    }

    fn validate_skill_output_schema(
        &self,
        job: &Job,
        envelope: &AgentResponseEnvelope,
    ) -> Result<(), OrbitError> {
        if job.target_type != JobTargetType::Work {
            return Ok(());
        }
        let work = self
            .context
            .store
            .get_work(&job.target_id)?
            .ok_or_else(|| OrbitError::WorkNotFound(job.target_id.clone()))?;
        let skills = self.resolve_work_skill_refs(&work.skill_refs)?;
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
        target_type: JobTargetType,
        target_id: &str,
    ) -> Result<(), OrbitError> {
        match target_type {
            JobTargetType::Work => {
                let Some(work) = self.context.store.get_work(target_id)? else {
                    return Err(OrbitError::WorkNotFound(target_id.to_string()));
                };
                let _ = self.resolve_work_skill_refs(&work.skill_refs)?;
            }
            JobTargetType::Workflow => {
                if self.context.store.get_workflow(target_id)?.is_none() {
                    return Err(OrbitError::WorkflowNotFound(target_id.to_string()));
                }
            }
        }
        Ok(())
    }
}
