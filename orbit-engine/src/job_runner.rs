use chrono::{DateTime, Utc};
use orbit_store::JobRunStepParams;
use orbit_types::{Job, JobRun, JobRunState, JobStep, OrbitError, OrbitEvent};
use serde_json::Value;

use crate::activity_runner::{build_execution_context_for_step, execute_with_retry};
use crate::context::{
    ACTIVITY_EXECUTION_FAILED, AGENT_INVOCATION_FAILED, EngineHost, JobRunHost, JobRunResult,
    RUN_ABANDONED, RuntimeHost, STALE_RUN_GRACE_SECONDS, step_output_for_following_input,
};

pub fn run_job_with_input<H: EngineHost>(
    host: &H,
    job: Job,
    input: Value,
    debug: bool,
) -> Result<JobRunResult, OrbitError> {
    let _ = recover_stale_active_run_for_job(host, &job, Utc::now())?;
    let active_runs = host.list_pending_or_running_job_runs(&job.job_id)?;
    if active_runs.len() as u32 >= job.max_active_runs {
        let latest_active_run = active_runs.first().expect("active run exists");
        return Err(OrbitError::JobValidation(format!(
            "job '{}' already has {} active run(s), reaching max_active_runs={} (latest active run '{}' in state '{}')",
            job.job_id,
            active_runs.len(),
            job.max_active_runs,
            latest_active_run.run_id,
            latest_active_run.state,
        )));
    }
    host.record_event(OrbitEvent::JobTriggered {
        job_id: job.job_id.clone(),
    })?;

    execute_activity_with_retries(host, job, Utc::now(), None, input, debug)
}

fn execute_activity_with_retries<H: EngineHost>(
    host: &H,
    job: Job,
    scheduled_at: DateTime<Utc>,
    initial_run: Option<JobRun>,
    input: Value,
    debug: bool,
) -> Result<JobRunResult, OrbitError> {
    let attempt = initial_run.as_ref().map(|r| r.attempt).unwrap_or(1);

    let mut run = if let Some(existing) = initial_run {
        existing
    } else {
        let run = host.insert_job_run(&job.job_id, attempt, scheduled_at)?;
        host.record_event(OrbitEvent::JobRunStarted {
            job_id: job.job_id.clone(),
            run_id: String::new(),
            attempt,
        })?;
        run
    };

    let started_at = Utc::now();
    let changed = host.mark_job_run_running(&run.run_id, started_at, std::process::id())?;
    if !changed {
        return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
    }
    host.record_event(OrbitEvent::JobRunStarted {
        job_id: job.job_id.clone(),
        run_id: run.run_id.clone(),
        attempt: run.attempt,
    })?;
    run.state = JobRunState::Running;
    run.started_at = Some(started_at);

    let default_failure_step =
        job.steps.first().cloned().ok_or_else(|| {
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
                build_execution_context_for_step(host, &job, step, current_input.clone(), debug)?;
            let step_started = Utc::now();
            let outcome = execute_with_retry(
                host,
                &execution,
                step.retry_max_attempts,
                step.retry_backoff_seconds,
            );
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
                for (key, value) in output_map {
                    input_map.insert(key.clone(), value.clone());
                }
            }

            let changed = host.complete_job_run_step(
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
        let duration_ms = (total_duration_ms > 0).then_some(total_duration_ms);

        let changed = host.finalize_job_run(&run.run_id, final_state, finished_at, duration_ms)?;
        if !changed {
            return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
        }
        host.record_event(OrbitEvent::JobRunCompleted {
            job_id: job.job_id.clone(),
            run_id: run.run_id.clone(),
            state: final_state.to_string(),
        })?;

        if last_protocol_violation {
            host.record_event(OrbitEvent::JobProtocolViolation {
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
            if let Some(active_run) = host.get_job_run(&run.run_id)?
                && matches!(
                    active_run.state,
                    JobRunState::Pending | JobRunState::Running
                )
            {
                let (step_index, step) = &failure_step;
                finalize_failed_started_run(host, &job, &run, *step_index, step, started_at, &err)?;
            }
            Err(err)
        }
    }
}

pub fn recover_stale_active_run_for_job<H: JobRunHost + RuntimeHost>(
    host: &H,
    job: &Job,
    now: DateTime<Utc>,
) -> Result<bool, OrbitError> {
    let active_runs = host.list_pending_or_running_job_runs(&job.job_id)?;
    if active_runs.is_empty() {
        return Ok(false);
    }
    let mut recovered_any = false;

    for active_run in active_runs {
        // PID-based abandonment: if the owning process has died, fail the run immediately.
        if let Some(pid) = active_run.pid
            && !pid_is_alive(pid)
        {
            eprintln!(
                "orbit: abandoning stale run '{}' (owner pid {} is no longer alive)",
                active_run.run_id, pid
            );
            let duration_ms = active_run.started_at.map(|started| {
                now.signed_duration_since(started).num_milliseconds().max(0) as u64
            });
            if let Some(first_step) = job.steps.first() {
                let _ = host.complete_job_run_step(
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
                        error_code: Some(RUN_ABANDONED.to_string()),
                        error_message: Some(format!(
                            "run abandoned: owner pid {} is no longer alive",
                            pid
                        )),
                    },
                );
            }
            let changed = host.abandon_job_run(&active_run.run_id, now)?;
            if !changed {
                return Err(OrbitError::JobRunNotFound(active_run.run_id.clone()));
            }
            host.record_event(OrbitEvent::JobRunCompleted {
                job_id: job.job_id.clone(),
                run_id: active_run.run_id.clone(),
                state: JobRunState::Failed.to_string(),
            })?;
            recovered_any = true;
            continue;
        }

        if !is_stale_active_run(job, &active_run, now) {
            continue;
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
            "stale active run recovered: run '{}' remained '{}' for {}s (timeout={}s, grace={}s)",
            active_run.run_id,
            active_run.state,
            age_seconds,
            total_timeout,
            STALE_RUN_GRACE_SECONDS
        );

        if let Some(first_step) = job.steps.first() {
            let _ = host.complete_job_run_step(
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

        let changed =
            host.finalize_job_run(&active_run.run_id, JobRunState::Failed, now, duration_ms)?;
        if !changed {
            return Err(OrbitError::JobRunNotFound(active_run.run_id.clone()));
        }
        host.record_event(OrbitEvent::JobRunCompleted {
            job_id: job.job_id.clone(),
            run_id: active_run.run_id.clone(),
            state: JobRunState::Failed.to_string(),
        })?;
        recovered_any = true;
    }

    Ok(recovered_any)
}

/// Returns `true` if the process with the given PID is currently alive.
/// On non-Unix platforms, conservatively returns `true` (never abandon).
#[cfg(unix)]
fn pid_is_alive(pid: u32) -> bool {
    // kill(pid, 0) checks process existence without sending a signal.
    // Returns 0 if the process exists, -1 with ESRCH if it does not.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn pid_is_alive(_pid: u32) -> bool {
    true
}

fn finalize_failed_started_run<H: JobRunHost + RuntimeHost>(
    host: &H,
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

    let changed = host.complete_job_run_step(
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

    let changed =
        host.finalize_job_run(&run.run_id, JobRunState::Failed, finished_at, duration_ms)?;
    if !changed {
        return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
    }
    host.record_event(OrbitEvent::JobRunCompleted {
        job_id: job.job_id.clone(),
        run_id: run.run_id.clone(),
        state: JobRunState::Failed.to_string(),
    })?;
    Ok(())
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
