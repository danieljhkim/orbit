use chrono::{DateTime, Utc};
use orbit_store::JobRunStepParams;
use orbit_types::{Job, JobRun, JobRunState, JobStep, OrbitError, OrbitEvent};
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::command::job::JobRunResult;
use crate::engine::{
    ACTIVITY_EXECUTION_FAILED, AGENT_INVOCATION_FAILED, STALE_RUN_GRACE_SECONDS,
    step_output_for_following_input,
};

impl OrbitRuntime {
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

        let default_failure_step = job.steps.first().cloned().ok_or_else(|| {
            OrbitError::JobValidation("job must have at least one step".to_string())
        })?;
        let mut failure_step = (0usize, default_failure_step);

        let execution_result: Result<JobRunResult, OrbitError> = (|| {
            let mut final_state = JobRunState::Success;
            let mut total_duration_ms: u64 = 0;
            let mut last_protocol_violation = false;
            let mut current_input = merge_job_input(job.default_input.as_ref(), input)?;

            for (step_index, step) in job.steps.iter().enumerate() {
                failure_step = (step_index, step.clone());
                let execution =
                    self.build_execution_context_for_step(&job, step, current_input.clone())?;
                let step_started = Utc::now();
                let outcome = self.execute_single_attempt(&execution);
                let step_finished = Utc::now();

                if let Some(d) = outcome.duration_ms {
                    total_duration_ms += d;
                }
                let step_state = outcome.state;

                if step_state == JobRunState::Success
                    && let Some(output_map) = step_output_for_following_input(
                        &execution.activity,
                        outcome.response_json.as_ref(),
                    )
                    && let Value::Object(ref mut input_map) = current_input
                {
                    for (k, v) in output_map {
                        input_map.insert(k.clone(), v.clone());
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

            Ok(JobRunResult {
                job_id: job.job_id.clone(),
                run_id: run.run_id.clone(),
                state: final_state,
                attempt: run.attempt,
            })
        })();

        match execution_result {
            Ok(result) => Ok(result),
            Err(err) => {
                if let Some(active_run) = self.context.job_store.get_job_run(&run.run_id)?
                    && matches!(
                        active_run.state,
                        JobRunState::Pending | JobRunState::Running
                    )
                {
                    let (step_index, step) = &failure_step;
                    self.finalize_failed_started_run(
                        &job,
                        &run,
                        *step_index,
                        step,
                        started_at,
                        &err,
                    )?;
                }
                Err(err)
            }
        }
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

    fn finalize_failed_started_run(
        &self,
        job: &Job,
        run: &JobRun,
        step_index: usize,
        step: &JobStep,
        started_at: DateTime<Utc>,
        err: &OrbitError,
    ) -> Result<(), OrbitError> {
        let finished_at = Utc::now();
        let duration_ms = Some(
            finished_at
                .signed_duration_since(started_at)
                .num_milliseconds()
                .max(0) as u64,
        );
        let message = err.to_string();

        let changed = self.complete_job_run_step_backend(
            &run.run_id,
            &JobRunStepParams {
                step_index,
                target_type: step.target_type,
                target_id: step.target_id.clone(),
                started_at,
                finished_at,
                duration_ms,
                exit_code: Some(1),
                agent_response_json: None,
                state: JobRunState::Failed,
                error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                error_message: Some(message.clone()),
            },
        )?;
        if !changed {
            return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
        }

        let changed = self.finalize_job_run_backend(
            &run.run_id,
            JobRunState::Failed,
            finished_at,
            duration_ms,
        )?;
        if !changed {
            return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
        }
        self.record_event(OrbitEvent::JobRunCompleted {
            job_id: job.job_id.clone(),
            run_id: run.run_id.clone(),
            state: JobRunState::Failed.to_string(),
        })?;
        Ok(())
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

fn merge_job_input(default_input: Option<&Value>, input: Value) -> Result<Value, OrbitError> {
    let mut merged = match default_input {
        None => serde_json::Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(other) => {
            return Err(OrbitError::InvalidInput(format!(
                "job default_input must be an object, got {}",
                json_value_type_name(other)
            )));
        }
    };

    let input_map = match input {
        Value::Object(map) => map,
        other => {
            return Err(OrbitError::InvalidInput(format!(
                "job run input must be an object, got {}",
                json_value_type_name(&other)
            )));
        }
    };

    for (key, value) in input_map {
        merged.insert(key, value);
    }

    Ok(Value::Object(merged))
}

fn json_value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
