use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_agent::{
    Agent, AgentConfig, AgentRequest, AgentResponseStatus, parse_and_validate_response,
};
use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_store::JobCreateParams as StoreActivityCreateParams;
use orbit_store::JobRunStepParams;
use orbit_types::{
    Activity, AgentCommitRequest, AgentResponseEnvelope, Job, JobRun, JobRunState,
    JobScheduleState, JobStep, JobTargetType, OrbitError, OrbitEvent,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tempfile::NamedTempFile;

use crate::OrbitRuntime;
use crate::command::activity::{activity_requires_agent_cli, activity_skill_refs_from_spec_config};
use crate::executor::{api, cli_command};
use crate::json_schema::validate_instance_against_schema;
use crate::paths;
use crate::template::TemplateContext;
const AGENT_PROTOCOL_VIOLATION: &str = "AGENT_PROTOCOL_VIOLATION";
const AGENT_INVOCATION_FAILED: &str = "AGENT_INVOCATION_FAILED";
const AGENT_COMMIT_FAILED: &str = "AGENT_COMMIT_FAILED";
const AGENT_TIMEOUT: &str = "AGENT_TIMEOUT";
const ACTIVITY_EXECUTION_FAILED: &str = "ACTIVITY_EXECUTION_FAILED";
const STALE_RUN_GRACE_SECONDS: u64 = 30;

#[derive(Debug, Clone)]
pub struct JobAddParams {
    pub job_id: Option<String>,
    pub steps: Vec<JobStep>,
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
    protocol_violation: bool,
    /// Validated absolute path to a report file the agent created, to be auto-committed
    /// after the run artifact is persisted.
    created_file: Option<String>,
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
    env_extra: Vec<String>,
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
        if params.steps.is_empty() {
            return Err(OrbitError::JobValidation(
                "job must have at least one step".to_string(),
            ));
        }

        for step in &params.steps {
            if step.target_id.trim().is_empty() {
                return Err(OrbitError::JobValidation(
                    "step target_id must not be empty".to_string(),
                ));
            }
            let activity =
                self.validate_activity_target_exists(step.target_type, &step.target_id)?;
            if activity_requires_agent_cli(&activity.spec_type) && step.agent_cli.trim().is_empty()
            {
                return Err(OrbitError::JobValidation(
                    "step agent_cli must not be empty for agent_invoke activities".to_string(),
                ));
            }
            // Validate runtime availability at add-time.
            if activity_requires_agent_cli(&activity.spec_type) {
                let _ = Agent::new(&AgentConfig::cli(step.agent_cli.clone()))?;
            }
        }

        let initial_state = params
            .initial_state_override
            .unwrap_or(JobScheduleState::Enabled);

        let steps = params
            .steps
            .into_iter()
            .map(|s| {
                let env_extra = crate::config::normalize_pass_list(s.env_extra)
                    .map_err(|e| OrbitError::JobValidation(e.to_string()))?;
                Ok(JobStep { env_extra, ..s })
            })
            .collect::<Result<Vec<_>, OrbitError>>()?;

        let job = self.context.job_store.add_job(StoreActivityCreateParams {
            job_id: params.job_id,
            steps,
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

    /// Returns jobs paired with their most recent run, if any.
    /// Recovers stale active runs before reading run history.
    pub fn list_jobs_with_last_run(
        &self,
        include_disabled: bool,
    ) -> Result<Vec<(Job, Option<JobRun>)>, OrbitError> {
        use orbit_store::JobRunQuery;

        let now = Utc::now();
        let jobs = self.list_jobs_backend(include_disabled)?;
        let mut result = Vec::with_capacity(jobs.len());
        for job in jobs {
            let _ = self.recover_stale_active_run_for_job(&job, now);
            let last_run = self
                .context
                .job_store
                .list_job_runs_filtered(&JobRunQuery {
                    job_id: Some(job.job_id.clone()),
                    state: None,
                    created_since: None,
                    limit: Some(1),
                })
                .ok()
                .and_then(|runs| runs.into_iter().next());
            result.push((job, last_run));
        }
        Ok(result)
    }

    pub fn show_job(&self, job_id: &str) -> Result<Job, OrbitError> {
        self.get_job_backend(job_id)?
            .ok_or_else(|| OrbitError::JobNotFound(job_id.to_string()))
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

    fn execute_activity_with_retries(
        &self,
        job: Job,
        scheduled_at: DateTime<Utc>,
        initial_run: Option<JobRun>,
        input: Value,
    ) -> Result<JobRunResult, OrbitError> {
        let attempt = initial_run.as_ref().map(|r| r.attempt).unwrap_or(1);

        let mut run = if let Some(existing) = initial_run {
            existing
        } else {
            let run = self.insert_job_run_backend(&job.job_id, attempt, scheduled_at)?;
            self.record_event(OrbitEvent::JobRunStarted {
                job_id: job.job_id.clone(),
                run_id: String::new(),
                attempt,
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

        let mut final_state = JobRunState::Success;
        let mut total_duration_ms: u64 = 0;
        let mut last_protocol_violation = false;
        let mut last_created_file: Option<String> = None;
        let mut current_input = input;

        for (step_index, step) in job.steps.iter().enumerate() {
            let execution =
                self.build_execution_context_for_step(&job, step, current_input.clone())?;
            let step_started = Utc::now();
            let outcome = self.execute_single_attempt(&execution);
            let step_finished = Utc::now();

            if let Some(d) = outcome.duration_ms {
                total_duration_ms += d;
            }
            let step_state = outcome.state;

            // Merge step output into current_input so subsequent steps receive
            // both the original inputs and any values produced by prior steps.
            if step_state == JobRunState::Success {
                if let Some(Value::Object(output_map)) = outcome.response_json.as_ref() {
                    if let Value::Object(ref mut input_map) = current_input {
                        for (k, v) in output_map {
                            input_map.insert(k.clone(), v.clone());
                        }
                    }
                }
            }

            let changed = self.complete_job_run_step_backend(
                &run.run_id,
                &JobRunStepParams {
                    step_index,
                    target_type: step.target_type,
                    target_id: step.target_id.clone(),
                    started_at: step_started,
                    finished_at: step_finished,
                    duration_ms: outcome.duration_ms,
                    exit_code: outcome.exit_code,
                    agent_response_json: outcome.response_json.clone(),
                    state: step_state,
                    error_code: outcome.error_code.clone(),
                    error_message: outcome.error_message.clone(),
                },
            )?;
            if !changed {
                return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
            }

            if outcome.protocol_violation {
                last_protocol_violation = true;
            }
            if let Some(f) = outcome.created_file {
                last_created_file = Some(f);
            }

            if step_state != JobRunState::Success {
                final_state = step_state;
                break;
            }
        }

        let finished_at = Utc::now();
        let duration_ms = if total_duration_ms > 0 {
            Some(total_duration_ms)
        } else {
            None
        };

        let changed =
            self.finalize_job_run_backend(&run.run_id, final_state, finished_at, duration_ms)?;
        if !changed {
            return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
        }
        self.record_event(OrbitEvent::JobRunCompleted {
            job_id: job.job_id.clone(),
            run_id: run.run_id.clone(),
            state: final_state.to_string(),
        })?;

        if last_protocol_violation {
            self.record_event(OrbitEvent::JobProtocolViolation {
                job_id: job.job_id.clone(),
                run_id: run.run_id.clone(),
                message: "agent protocol violation".to_string(),
            })?;
        }

        if final_state == JobRunState::Success {
            if let Some(ref created_file_path) = last_created_file {
                self.execute_created_file_auto_commit(created_file_path, &job.job_id, &run.run_id)?;
            }
        }

        Ok(JobRunResult {
            job_id: job.job_id.clone(),
            run_id: run.run_id.clone(),
            state: final_state,
            attempt: run.attempt,
        })
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
        let total_timeout: u64 = job.steps.iter().map(|s| s.timeout_seconds).sum();
        let message = format!(
            "stale active run recovered: run '{}' remained '{}' for {}s \
(timeout={}s, grace={}s)",
            active_run.run_id,
            active_run.state,
            age_seconds,
            total_timeout,
            STALE_RUN_GRACE_SECONDS
        );

        // Write a synthetic step file so error_code/error_message are retrievable.
        if let Some(first_step) = job.steps.first() {
            let _ = self.complete_job_run_step_backend(
                &active_run.run_id,
                &JobRunStepParams {
                    step_index: 0,
                    target_type: first_step.target_type,
                    target_id: first_step.target_id.clone(),
                    started_at: active_run.started_at.unwrap_or(active_run.created_at),
                    finished_at: now,
                    duration_ms,
                    exit_code: Some(1),
                    agent_response_json: None,
                    state: JobRunState::Failed,
                    error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
                    error_message: Some(message.clone()),
                },
            );
        }

        let changed = self.finalize_job_run_backend(
            &active_run.run_id,
            JobRunState::Failed,
            now,
            duration_ms,
        )?;
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
            env_extra: vec![],
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

    fn build_execution_context_for_step(
        &self,
        job: &Job,
        step: &JobStep,
        input: Value,
    ) -> Result<ExecutionContext, OrbitError> {
        let activity = self.show_activity(&step.target_id)?;
        self.validate_activity_input_schema(&activity, &input)?;
        Ok(ExecutionContext {
            activity,
            job: Some(job.clone()),
            agent_cli: step.agent_cli.clone(),
            timeout_seconds: step.timeout_seconds,
            env_extra: step.env_extra.clone(),
            input,
        })
    }

    fn execute_single_attempt(&self, execution: &ExecutionContext) -> AttemptOutcome {
        match execution.activity.spec_type.as_str() {
            "agent_invoke" => self.execute_agent_attempt(execution),
            "cli_command" => self.execute_cli_command_attempt(execution),
            "api" => self.execute_api_attempt(execution),
            other => AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: Some(1),
                duration_ms: None,
                response_json: None,
                error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                error_message: Some(format!("unsupported activity spec_type '{other}'")),
                protocol_violation: false,
                created_file: None,
            },
        }
    }

    fn execute_agent_attempt(&self, execution: &ExecutionContext) -> AttemptOutcome {
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
                    protocol_violation: false,
                    created_file: None,
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
                    protocol_violation: false,
                    created_file: None,
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
                    protocol_violation: false,
                    created_file: None,
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
                protocol_violation: false,
                created_file: None,
            };
        };
        let environment_mode = if self.context.execution_env_policy.inherit() {
            EnvironmentMode::Inherit
        } else {
            EnvironmentMode::ClearAndSet(
                self.context
                    .execution_env_policy
                    .hydrated_allowlist_env_with_extras(&execution.env_extra),
            )
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
                    protocol_violation: false,
                    created_file: None,
                };
            }
        };

        let exec_result = match run_process(
            &ExecRequest {
                program: invocation.program,
                args,
                current_dir: None,
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
                    protocol_violation: false,
                    created_file: None,
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
                protocol_violation: false,
                created_file: None,
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
                        protocol_violation: true,
                        created_file: None,
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
                        protocol_violation,
                        created_file: None,
                    };
                }
                // Validate any `created_file` path returned by the agent before persisting the
                // run. Invalid paths fail the run as a protocol violation so no unintended files
                // are staged. The validated absolute path is carried forward; the actual git
                // commit runs after run persistence so the run artifact can be included.
                let validated_created_file = if run_state == JobRunState::Success {
                    match envelope.result.as_ref().and_then(|r| r.get("created_file")) {
                        Some(Value::String(s)) => match self.validate_created_file_path(s) {
                            Ok(abs_path) => Some(abs_path.to_string_lossy().into_owned()),
                            Err(OrbitError::AgentProtocolViolation(msg)) => {
                                return AttemptOutcome {
                                    state: JobRunState::Failed,
                                    exit_code: exec_result.exit_code,
                                    duration_ms: Some(exec_result.duration_ms),
                                    response_json: None,
                                    error_code: Some(AGENT_PROTOCOL_VIOLATION.to_string()),
                                    error_message: Some(msg),
                                    protocol_violation: true,
                                    created_file: None,
                                };
                            }
                            Err(err) => {
                                return AttemptOutcome {
                                    state: JobRunState::Failed,
                                    exit_code: exec_result.exit_code,
                                    duration_ms: Some(exec_result.duration_ms),
                                    response_json: None,
                                    error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
                                    error_message: Some(err.to_string()),
                                    protocol_violation: false,
                                    created_file: None,
                                };
                            }
                        },
                        Some(_) => {
                            return AttemptOutcome {
                                state: JobRunState::Failed,
                                exit_code: exec_result.exit_code,
                                duration_ms: Some(exec_result.duration_ms),
                                response_json: None,
                                error_code: Some(AGENT_PROTOCOL_VIOLATION.to_string()),
                                error_message: Some(
                                    "result.created_file must be a string".to_string(),
                                ),
                                protocol_violation: true,
                                created_file: None,
                            };
                        }
                        None => None,
                    }
                } else {
                    None
                };
                AttemptOutcome {
                    state: run_state,
                    exit_code: exec_result.exit_code,
                    duration_ms: Some(exec_result.duration_ms),
                    response_json: serde_json::to_value(envelope).ok(),
                    error_code,
                    error_message,
                    protocol_violation: false,
                    created_file: validated_created_file,
                }
            }
            Err(OrbitError::AgentProtocolViolation(message)) => AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: exec_result.exit_code,
                duration_ms: Some(exec_result.duration_ms),
                response_json: None,
                error_code: Some(AGENT_PROTOCOL_VIOLATION.to_string()),
                error_message: Some(message),
                protocol_violation: true,
                created_file: None,
            },
            Err(err) => AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: exec_result.exit_code,
                duration_ms: Some(exec_result.duration_ms),
                response_json: None,
                error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
                error_message: Some(err.to_string()),
                protocol_violation: false,
                created_file: None,
            },
        }
    }

    fn execute_cli_command_attempt(&self, execution: &ExecutionContext) -> AttemptOutcome {
        let template_context = self.execution_template_context(execution);
        match cli_command::execute(
            &execution.activity.spec_config,
            &template_context,
            execution.timeout_seconds,
        ) {
            Ok((result, duration_ms, exit_code)) => {
                if let Err(err) = self.validate_activity_output_schema(&execution.activity, &result)
                {
                    return AttemptOutcome {
                        state: JobRunState::Failed,
                        exit_code,
                        duration_ms: Some(duration_ms),
                        response_json: Some(result),
                        error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                        error_message: Some(err.to_string()),
                        protocol_violation: false,
                        created_file: None,
                    };
                }
                AttemptOutcome {
                    state: JobRunState::Success,
                    exit_code,
                    duration_ms: Some(duration_ms),
                    response_json: Some(result),
                    error_code: None,
                    error_message: None,
                    protocol_violation: false,
                    created_file: None,
                }
            }
            Err(err) => AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: Some(1),
                duration_ms: None,
                response_json: None,
                error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                error_message: Some(err.to_string()),
                protocol_violation: false,
                created_file: None,
            },
        }
    }

    fn execute_api_attempt(&self, execution: &ExecutionContext) -> AttemptOutcome {
        let template_context = self.execution_template_context(execution);
        match api::execute(
            &execution.activity.spec_config,
            &template_context,
            execution.timeout_seconds,
        ) {
            Ok(result) => {
                if let Err(err) = self.validate_activity_output_schema(&execution.activity, &result)
                {
                    return AttemptOutcome {
                        state: JobRunState::Failed,
                        exit_code: Some(0),
                        duration_ms: None,
                        response_json: Some(result),
                        error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                        error_message: Some(err.to_string()),
                        protocol_violation: false,
                        created_file: None,
                    };
                }
                AttemptOutcome {
                    state: JobRunState::Success,
                    exit_code: Some(0),
                    duration_ms: None,
                    response_json: Some(result),
                    error_code: None,
                    error_message: None,
                    protocol_violation: false,
                    created_file: None,
                }
            }
            Err(err) => AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: Some(1),
                duration_ms: None,
                response_json: None,
                error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                error_message: Some(err.to_string()),
                protocol_violation: false,
                created_file: None,
            },
        }
    }

    fn build_stdin_envelope_payload(
        &self,
        execution: &ExecutionContext,
    ) -> Result<Vec<u8>, OrbitError> {
        let skill_refs = activity_skill_refs_from_spec_config(&execution.activity.spec_config)?;
        let skills = self.resolve_activity_skill_refs(&skill_refs)?;
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
            activity: activity_envelope_json(&execution.activity),
            job: execution.job.as_ref().map(|job| {
                json!({
                    "id": job.job_id,
                    "state": job.state,
                    "steps": job.steps.iter().map(|s| json!({
                        "target_type": s.target_type,
                        "target_id": s.target_id,
                        "agent_cli": s.agent_cli,
                        "timeout_seconds": s.timeout_seconds,
                    })).collect::<Vec<_>>(),
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
        let skill_refs = activity_skill_refs_from_spec_config(&activity.spec_config)?;
        let skills = self.resolve_activity_skill_refs(&skill_refs)?;
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

    fn execution_template_context(&self, execution: &ExecutionContext) -> TemplateContext {
        TemplateContext {
            input: execution.input.clone(),
            env: std::env::vars().collect(),
            workspace_path: execution.activity.workspace_path.clone(),
        }
    }

    fn validate_activity_output_schema(
        &self,
        activity: &Activity,
        output: &Value,
    ) -> Result<(), OrbitError> {
        let context = format!(
            "activity '{}' output does not match output schema",
            activity.id
        );
        validate_instance_against_schema(&activity.output_schema_json, output, &context)
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

    /// Validate a `created_file` path returned by an agent.
    ///
    /// Returns the canonical absolute path on success. Fails with
    /// `AgentProtocolViolation` for agent errors (empty path, missing file,
    /// path outside repo root) so the run is marked as a protocol violation.
    fn validate_created_file_path(&self, path: &str) -> Result<PathBuf, OrbitError> {
        let path = path.trim();
        if path.is_empty() {
            return Err(OrbitError::AgentProtocolViolation(
                "result.created_file must not be empty".to_string(),
            ));
        }

        let repo_root = paths::find_git_repo_root(&self.context.data_root).ok_or_else(|| {
            OrbitError::AgentProtocolViolation(format!(
                "result.created_file: cannot locate git repository root from Orbit data root '{}'",
                self.context.data_root.display()
            ))
        })?;

        // Resolve to absolute path (relative paths are resolved relative to repo root).
        let abs_path = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            repo_root.join(path)
        };

        if !abs_path.exists() {
            return Err(OrbitError::AgentProtocolViolation(format!(
                "result.created_file '{}' does not exist",
                path
            )));
        }

        let canonical = abs_path.canonicalize().map_err(|e| {
            OrbitError::AgentProtocolViolation(format!(
                "result.created_file '{}' cannot be resolved: {e}",
                path
            ))
        })?;
        let canonical_root = repo_root.canonicalize().map_err(|e| {
            OrbitError::Execution(format!(
                "failed to canonicalize repo root '{}': {e}",
                repo_root.display()
            ))
        })?;

        if !canonical.starts_with(&canonical_root) {
            return Err(OrbitError::AgentProtocolViolation(format!(
                "result.created_file '{}' is outside the git repository root",
                path
            )));
        }

        Ok(canonical)
    }

    /// Commit the agent-created report file together with the current run artifact.
    ///
    /// This is called AFTER `complete_job_run_backend` so the run YAML exists on disk
    /// and can be included in the same audited commit as the report.
    fn execute_created_file_auto_commit(
        &self,
        created_file: &str, // canonical absolute path (already validated)
        job_id: &str,
        run_id: &str,
    ) -> Result<(), OrbitError> {
        let repo_root = paths::find_git_repo_root(&self.context.data_root).ok_or_else(|| {
            OrbitError::Execution(format!(
                "cannot locate git repository root from Orbit data root '{}'",
                self.context.data_root.display()
            ))
        })?;
        let canonical_root = repo_root.canonicalize().map_err(|e| {
            OrbitError::Execution(format!(
                "failed to canonicalize repo root '{}': {e}",
                repo_root.display()
            ))
        })?;

        // Convert created_file to repo-root-relative path for git.
        let created_file_abs = PathBuf::from(created_file);
        let created_file_rel = created_file_abs
            .strip_prefix(&canonical_root)
            .map_err(|_| {
                OrbitError::Execution(format!(
                    "created_file '{}' is outside repo root '{}'",
                    created_file,
                    canonical_root.display()
                ))
            })?
            .to_string_lossy()
            .into_owned();

        let mut files = vec![created_file_rel];

        // Include the run artifact YAML when file-based job persistence is used.
        // This is only possible after finalize_job_run_backend has persisted the run.
        let run_artifact_abs = self
            .context
            .persistence
            .job
            .path
            .join("runs")
            .join(job_id)
            .join(run_id)
            .join("jrun.yaml");
        if run_artifact_abs.exists() {
            if let Ok(canonical_artifact) = run_artifact_abs.canonicalize() {
                if let Ok(rel) = canonical_artifact.strip_prefix(&canonical_root) {
                    files.push(rel.to_string_lossy().into_owned());
                }
            }
        }

        let message = format!("orbit: {job_id} run artifact and report [{run_id}]");
        let repo_root_str = canonical_root.to_string_lossy().to_string();

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
                "repo_root": repo_root_str,
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
    ) -> Result<Activity, OrbitError> {
        let _ = target_type;
        let activity = self.show_activity(target_id)?;
        let skill_refs = activity_skill_refs_from_spec_config(&activity.spec_config)?;
        let _ = self.resolve_activity_skill_refs(&skill_refs)?;
        Ok(activity)
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

    fn complete_job_run_step_backend(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.context.job_store.complete_job_run_step(run_id, params)
    }

    fn finalize_job_run_backend(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.context
            .job_store
            .finalize_job_run(run_id, state, finished_at, duration_ms)
    }
}

fn is_stale_active_run(job: &Job, run: &JobRun, now: DateTime<Utc>) -> bool {
    let total_timeout: u64 = job.steps.iter().map(|s| s.timeout_seconds).sum();
    let reference_time = run.started_at.unwrap_or(run.created_at);
    let elapsed_seconds = now.signed_duration_since(reference_time).num_seconds();
    let stale_after_seconds = total_timeout.saturating_add(STALE_RUN_GRACE_SECONDS) as i64;
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

fn activity_envelope_json(activity: &Activity) -> Value {
    let mut envelope = json!({
        "id": activity.id,
        "type": activity.spec_type,
        "description": activity.description,
        "input_schema_json": activity.input_schema_json,
        "output_schema_json": activity.output_schema_json,
        "identity_id": activity.identity_id,
        "created_by": activity.created_by,
    });

    if let Some(activity_map) = envelope.as_object_mut()
        && let Some(spec_config) = activity.spec_config.as_object()
    {
        for (key, value) in spec_config {
            activity_map.insert(key.clone(), value.clone());
        }
    }

    envelope
}

const DEFAULT_JOB_FILES: &[(&str, &str)] = &[
    (
        "job_review_tasks",
        include_str!("../../assets/jobs/job_review_tasks.yaml"),
    ),
    (
        "job_oversee_orbit_operations",
        include_str!("../../assets/jobs/job_oversee_orbit_operations.yaml"),
    ),
    (
        "job_perform_maintenance",
        include_str!("../../assets/jobs/job_perform_maintenance.yaml"),
    ),
];

#[derive(Debug, Clone, Deserialize)]
struct DefaultJobFileSpec {
    job: DefaultJobEntry,
}

#[derive(Debug, Clone, Deserialize)]
struct DefaultJobEntry {
    job_id: String,
    state: String,
    steps: Vec<DefaultJobStep>,
}

#[derive(Debug, Clone, Deserialize)]
struct DefaultJobStep {
    target_type: String,
    target_id: String,
    agent_cli: String,
    timeout_seconds: u64,
    #[serde(default)]
    env_extra: Vec<String>,
}

fn load_default_job_specs(raw_specs: &[(&str, &str)]) -> Result<Vec<DefaultJobEntry>, OrbitError> {
    let mut specs = Vec::with_capacity(raw_specs.len());
    for (expected_id, raw) in raw_specs {
        let file_spec = serde_yaml::from_str::<DefaultJobFileSpec>(raw).map_err(|err| {
            OrbitError::InvalidInput(format!("invalid default job spec '{}': {err}", expected_id))
        })?;
        let entry = file_spec.job;
        let id = entry.job_id.trim();
        if id != *expected_id {
            return Err(OrbitError::InvalidInput(format!(
                "default job file key '{}' does not match spec job_id '{}'",
                expected_id, id
            )));
        }
        specs.push(entry);
    }
    Ok(specs)
}

pub(crate) fn seed_default_jobs(runtime: &OrbitRuntime) -> Result<usize, OrbitError> {
    let specs = load_default_job_specs(DEFAULT_JOB_FILES)?;
    let mut created = 0usize;
    for entry in specs {
        if runtime.show_job(&entry.job_id).is_ok() {
            continue;
        }
        let initial_state = match entry.state.as_str() {
            "enabled" => JobScheduleState::Enabled,
            "disabled" => JobScheduleState::Disabled,
            other => {
                return Err(OrbitError::InvalidInput(format!(
                    "unsupported state '{}' in default job '{}'",
                    other, entry.job_id
                )));
            }
        };
        let steps = entry
            .steps
            .into_iter()
            .map(|s| {
                let target_type = match s.target_type.as_str() {
                    "activity" => JobTargetType::Activity,
                    other => {
                        return Err(OrbitError::InvalidInput(format!(
                            "unsupported target_type '{}' in default job '{}'",
                            other, entry.job_id
                        )));
                    }
                };
                Ok(JobStep {
                    target_type,
                    target_id: s.target_id,
                    agent_cli: s.agent_cli,
                    timeout_seconds: s.timeout_seconds,
                    env_extra: s.env_extra,
                })
            })
            .collect::<Result<Vec<_>, OrbitError>>()?;
        runtime.add_job(JobAddParams {
            job_id: Some(entry.job_id),
            steps,
            initial_state_override: Some(initial_state),
        })?;
        created += 1;
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_JOB_FILES, load_default_job_specs};

    #[test]
    fn bundled_default_job_specs_parse_successfully() {
        let specs =
            load_default_job_specs(DEFAULT_JOB_FILES).expect("bundled default jobs must parse");
        assert_eq!(specs.len(), DEFAULT_JOB_FILES.len());
        for spec in &specs {
            assert!(!spec.job_id.is_empty(), "job_id must not be empty");
            assert!(!spec.steps.is_empty(), "steps must not be empty");
            for step in &spec.steps {
                assert!(!step.target_id.is_empty(), "target_id must not be empty");
                assert!(!step.agent_cli.is_empty(), "agent_cli must not be empty");
                assert!(step.timeout_seconds > 0, "timeout_seconds must be positive");
            }
        }
    }

    #[test]
    fn load_rejects_mismatched_file_key_and_job_id() {
        let specs = &[(
            "expected-id",
            "job:\n  job_id: actual-id\n  state: enabled\n  steps:\n    - target_type: activity\n      target_id: t\n      agent_cli: codex\n      timeout_seconds: 60\n",
        )];
        let err = load_default_job_specs(specs).expect_err("must fail");
        assert!(err.to_string().contains("does not match spec job_id"));
    }
}
