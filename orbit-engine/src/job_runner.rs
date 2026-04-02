use chrono::{DateTime, Utc};
use orbit_agent::Agent;
use orbit_store::JobRunStepParams;
use orbit_store::friction_log::append_friction_entry;
use orbit_store::metrics_log::append_metrics_entry;
use orbit_types::{
    ActorIdentity, FrictionEntry, Job, JobRun, JobRunState, JobStep, JobTargetType, MetricsEntry,
    OrbitError, OrbitEvent, StepCondition,
};
use serde_json::Value;
use std::path::Path;
#[cfg(unix)]
use std::process::Command;
use tracing::{error, info, info_span, warn};

use crate::activity_runner::{build_execution_context_for_step, execute_with_retry};
use crate::context::{
    ACTIVITY_EXECUTION_FAILED, AGENT_INVOCATION_FAILED, EngineHost, JobRunHost, JobRunResult,
    RUN_ABANDONED, RuntimeHost, STALE_RUN_GRACE_SECONDS, step_output_for_following_input,
};

pub fn run_job_with_input<H: EngineHost>(
    host: &H,
    data_root: &Path,
    job: Job,
    input: Value,
    debug: bool,
) -> Result<JobRunResult, OrbitError> {
    let job_span = info_span!("job_dispatch", job_id = %job.job_id);
    let _job_span = job_span.enter();
    info!(max_active_runs = job.max_active_runs, "job run requested");
    let _ = host.cleanup_stale_file_locks()?;
    let _ = recover_stale_active_run_for_job(host, data_root, &job, Utc::now())?;
    let active_runs = host.list_pending_or_running_job_runs(&job.job_id)?;
    if active_runs.len() as u32 >= job.max_active_runs {
        let latest_active_run = active_runs.first().ok_or_else(|| {
            OrbitError::JobValidation(format!(
                "job '{}' has no active runs despite reaching max_active_runs={}",
                job.job_id, job.max_active_runs
            ))
        })?;
        warn!(
            active_run_count = active_runs.len(),
            latest_active_run_id = %latest_active_run.run_id,
            latest_active_run_state = %latest_active_run.state,
            max_active_runs = job.max_active_runs,
            "job run rejected because max_active_runs was reached"
        );
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

    execute_activity_with_retries(
        host,
        data_root,
        job,
        ActivityExecutionRequest {
            scheduled_at: Utc::now(),
            initial_run: None,
            input: input.clone(),
            debug,
            create_failure_task: true,
            skip_to_step: 0,
            replayed_steps: &[],
        },
    )
}

/// Resume a failed job run from a specific step.
///
/// Creates a NEW run (preserving audit trail). Steps before `retry_step_target_id`
/// are written as Skipped records with replayed outputs from the source run.
/// Execution resumes from the specified step.
pub fn retry_job_run_from_step<H: EngineHost>(
    host: &H,
    data_root: &Path,
    job: Job,
    source_run: JobRun,
    retry_step_target_id: &str,
    debug: bool,
) -> Result<JobRunResult, OrbitError> {
    let job_span = info_span!(
        "job_retry",
        job_id = %job.job_id,
        source_run_id = %source_run.run_id,
        retry_step_target_id
    );
    let _job_span = job_span.enter();
    info!("job run retry requested");
    let _ = host.cleanup_stale_file_locks()?;
    // Find the step index to retry from (in the job definition, not the run steps).
    // Only supports first iteration (iteration 0) for v1.
    let retry_from_index = job
        .steps
        .iter()
        .position(|s| s.target_id == retry_step_target_id)
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "step '{}' not found in job '{}' definition",
                retry_step_target_id, job.job_id
            ))
        })?;

    // Reconstruct the input from the source run's persisted input, falling back
    // to an empty object merged with defaults.
    let base_input = source_run
        .input
        .clone()
        .unwrap_or_else(|| Value::Object(Default::default()));

    // Recover any stale runs before creating a new one
    let _ = recover_stale_active_run_for_job(host, data_root, &job, Utc::now())?;

    // Check max_active_runs
    let active_runs = host.list_pending_or_running_job_runs(&job.job_id)?;
    if active_runs.len() as u32 >= job.max_active_runs {
        warn!(
            active_run_count = active_runs.len(),
            max_active_runs = job.max_active_runs,
            "job retry rejected because max_active_runs was reached"
        );
        return Err(OrbitError::JobValidation(format!(
            "job '{}' already has {} active run(s), reaching max_active_runs={}",
            job.job_id,
            active_runs.len(),
            job.max_active_runs
        )));
    }

    host.record_event(OrbitEvent::JobTriggered {
        job_id: job.job_id.clone(),
    })?;

    let now = Utc::now();

    execute_activity_with_retries(
        host,
        data_root,
        job,
        ActivityExecutionRequest {
            scheduled_at: now,
            initial_run: None,
            input: base_input,
            debug,
            create_failure_task: true,
            skip_to_step: retry_from_index,
            replayed_steps: &source_run.steps,
        },
    )
}

struct ActivityExecutionRequest<'a> {
    scheduled_at: DateTime<Utc>,
    initial_run: Option<JobRun>,
    input: Value,
    debug: bool,
    // When `true`, a failure task is created on pipeline failure.
    // Nested (sub-job) runs pass `false` so only the outermost pipeline
    // creates a single failure task.
    create_failure_task: bool,
    // When > 0, steps before this index are written as Skipped records with
    // replayed data from the source run. Execution starts from this index.
    skip_to_step: usize,
    // Source run steps used to replay data when `skip_to_step > 0`.
    replayed_steps: &'a [orbit_types::JobRunStep],
}

fn execute_activity_with_retries<H: EngineHost>(
    host: &H,
    data_root: &Path,
    job: Job,
    request: ActivityExecutionRequest<'_>,
) -> Result<JobRunResult, OrbitError> {
    let ActivityExecutionRequest {
        scheduled_at,
        initial_run,
        input,
        debug,
        create_failure_task,
        skip_to_step,
        replayed_steps,
    } = request;
    let attempt = initial_run.as_ref().map(|r| r.attempt).unwrap_or(1);

    let mut run = if let Some(existing) = initial_run {
        existing
    } else {
        let run = host.insert_job_run(
            &job.job_id,
            attempt,
            scheduled_at,
            Some(input.clone()),
            None,
        )?;
        host.record_event(OrbitEvent::JobRunStarted {
            job_id: job.job_id.clone(),
            run_id: String::new(),
            attempt,
        })?;
        info!(
            run_id = %run.run_id,
            attempt,
            scheduled_at = %scheduled_at,
            "job run created"
        );
        run
    };

    let run_span = info_span!(
        "job_run",
        job_id = %job.job_id,
        run_id = %run.run_id,
        attempt = run.attempt
    );
    let _run_span = run_span.enter();

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
        let mut current_input = merge_job_input(job.default_input.as_ref(), input.clone())?;
        // Inject run_id so all steps can reference it (e.g. as batch_id for
        // parallel task pipelines).
        if let Value::Object(ref mut map) = current_input {
            map.entry("run_id")
                .or_insert_with(|| Value::String(run.run_id.clone()));
        }
        let mut last_failure: Option<FailureInfo> = None;
        let num_steps = job.steps.len();
        let max_iterations = job.max_iterations.max(1);

        'outer: for iteration in 0..max_iterations {
            let mut previous_step_state: Option<JobRunState> = None;

            // Clear loop_exit flag at the start of each iteration so a
            // previous iteration's flag doesn't short-circuit immediately.
            if iteration > 0
                && let Value::Object(ref mut map) = current_input
            {
                map.remove("loop_exit");
            }

            for (step_index, step) in job.steps.iter().enumerate() {
                if run_was_cancelled(host, &run.run_id)? {
                    final_state = JobRunState::Cancelled;
                    break 'outer;
                }

                let global_step_index = iteration as usize * num_steps + step_index;

                // When retrying from a specific step, skip earlier steps by writing
                // Skipped records with replayed data from the source run.
                if global_step_index < skip_to_step {
                    let skipped_at = Utc::now();
                    let source_step = replayed_steps
                        .iter()
                        .find(|s| s.step_index as usize == global_step_index);

                    // Replay successful step outputs into current_input
                    if let Some(src) = source_step {
                        if src.state == JobRunState::Success {
                            let activity = host.validate_activity_target_exists(
                                step.target_type,
                                &step.target_id,
                            )?;
                            if let Some(output_map) = step_output_for_following_input(
                                &activity,
                                src.agent_response_json.as_ref(),
                            ) && let Value::Object(ref mut input_map) = current_input
                            {
                                let mut merged: serde_json::Map<String, Value> = output_map
                                    .iter()
                                    .map(|(k, v)| (k.clone(), v.clone()))
                                    .collect();
                                for (source_key, target_key) in &step.output_map {
                                    if let Some(value) = merged.remove(source_key) {
                                        merged.insert(target_key.clone(), value);
                                    }
                                }
                                for (key, value) in merged {
                                    input_map.insert(key, value);
                                }
                            }
                        }
                        if src.state != JobRunState::Skipped {
                            previous_step_state = Some(src.state);
                        }
                    }

                    host.complete_job_run_step(
                        &run.run_id,
                        &JobRunStepParams {
                            step_index: global_step_index,
                            target_type: step.target_type,
                            target_id: step.target_id.clone(),
                            started_at: skipped_at,
                            finished_at: skipped_at,
                            duration_ms: Some(0),
                            exit_code: source_step.and_then(|s| s.exit_code),
                            agent_response_json: source_step
                                .and_then(|s| s.agent_response_json.clone()),
                            state: JobRunState::Skipped,
                            error_code: None,
                            error_message: Some("replayed from source run".to_string()),
                        },
                    )?;
                    continue;
                }

                failure_step = (global_step_index, step.clone());

                if !should_run_step(step.condition, previous_step_state) {
                    let skipped_at = Utc::now();
                    let changed = host.complete_job_run_step(
                        &run.run_id,
                        &JobRunStepParams {
                            step_index: global_step_index,
                            target_type: step.target_type,
                            target_id: step.target_id.clone(),
                            started_at: skipped_at,
                            finished_at: skipped_at,
                            duration_ms: Some(0),
                            exit_code: None,
                            agent_response_json: None,
                            state: JobRunState::Skipped,
                            error_code: None,
                            error_message: None,
                        },
                    )?;
                    if !changed {
                        return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
                    }
                    // Pass-through: do NOT update previous_step_state when skipped.
                    // This makes skipped steps transparent so subsequent conditions
                    // see the last non-skipped step's state, enabling patterns like
                    // on_failure branch → on_success continuation.
                    continue;
                }

                info!(
                    step_index = global_step_index,
                    iteration,
                    target_id = %step.target_id,
                    target_type = %step.target_type,
                    "step started"
                );

                // ---- Job-as-step: delegate to a nested job run ----
                if step.target_type == JobTargetType::Job {
                    let step_started = Utc::now();
                    let sub_result =
                        execute_job_step(host, data_root, &step.target_id, &current_input, debug);
                    let step_finished = Utc::now();
                    let (step_state, duration_ms, error_code, error_message) = match &sub_result {
                        Ok(result) => (result.state, None, None, None),
                        Err(err) => (
                            JobRunState::Failed,
                            None,
                            Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                            Some(err.to_string()),
                        ),
                    };
                    // Cancelled is not a valid step result state; map it to Failed
                    // so validate_step_state() accepts it when persisting.
                    let step_state = if step_state == JobRunState::Cancelled {
                        JobRunState::Failed
                    } else {
                        step_state
                    };
                    previous_step_state = Some(step_state);

                    let changed = host.complete_job_run_step(
                        &run.run_id,
                        &JobRunStepParams {
                            step_index: global_step_index,
                            target_type: step.target_type,
                            target_id: step.target_id.clone(),
                            started_at: step_started,
                            finished_at: step_finished,
                            duration_ms,
                            exit_code: None,
                            agent_response_json: None,
                            state: step_state,
                            error_code: error_code.clone(),
                            error_message: error_message.clone(),
                        },
                    )?;
                    if !changed {
                        return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
                    }

                    log_step_completion(
                        global_step_index,
                        iteration,
                        step,
                        step_state,
                        duration_ms,
                        error_code.as_deref(),
                        error_message.as_deref(),
                    );

                    if step_state_records_incident(step_state) {
                        // Preserve the first failure — subsequent handler failures
                        // should not overwrite the original root cause.
                        if last_failure.is_none() {
                            last_failure = Some(FailureInfo {
                                error_code: error_code.unwrap_or_default(),
                                error_message: error_message.unwrap_or_default(),
                                agent: (!step.agent_cli.trim().is_empty())
                                    .then(|| normalize_agent_label(&step.agent_cli)),
                                model: step.model.clone(),
                            });
                        }
                        final_state = step_state;
                    }
                    continue;
                }

                // ---- Activity step (existing behavior) ----
                // If the step's agent_cli is empty, try to resolve it from the
                // task's agent/model fields so the original implementer is used.
                let resolved_step = resolve_step_agent_from_task(host, step, &current_input);
                let effective_step = resolved_step.as_ref().unwrap_or(step);
                let execution = build_execution_context_for_step(
                    host,
                    &job,
                    effective_step,
                    current_input.clone(),
                    debug,
                )?;
                // Only record agent context when the step explicitly specifies
                // agent_cli — skip when resolved from the task to avoid overwriting.
                if !step.agent_cli.trim().is_empty() {
                    record_task_agent_context(host, &execution)?;
                }
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
                // Cancelled is not a valid step result state; map it to Failed
                // so validate_step_state() accepts it when persisting.
                let step_state = if outcome.state == JobRunState::Cancelled {
                    JobRunState::Failed
                } else {
                    outcome.state
                };
                previous_step_state = Some(step_state);

                // Pipe this step's output fields into the next step's input.
                if step_state == JobRunState::Success
                    && let Some(output_map) = step_output_for_following_input(
                        &execution.activity,
                        outcome.response_json.as_ref(),
                    )
                    && let Value::Object(ref mut input_map) = current_input
                {
                    let mut merged: serde_json::Map<String, Value> = output_map
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    for (source, target) in &step.output_map {
                        if let Some(value) = merged.remove(source) {
                            merged.insert(target.clone(), value);
                        }
                    }
                    for (key, value) in merged {
                        input_map.insert(key, value);
                    }
                }

                let changed = host.complete_job_run_step(
                    &run.run_id,
                    &JobRunStepParams {
                        step_index: global_step_index,
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

                log_step_completion(
                    global_step_index,
                    iteration,
                    step,
                    step_state,
                    outcome.duration_ms,
                    outcome.error_code.as_deref(),
                    outcome.error_message.as_deref(),
                );

                if step_state_records_incident(step_state) {
                    append_failed_step_friction(
                        data_root,
                        host,
                        &run.run_id,
                        &step.target_id,
                        &execution,
                        outcome.exit_code,
                        outcome.error_message.as_deref().unwrap_or(""),
                        step_finished,
                    );
                }

                append_step_metrics(
                    data_root,
                    host,
                    &run.run_id,
                    &step.target_id,
                    &execution,
                    outcome.duration_ms,
                    outcome.retry_count,
                    step_finished,
                );

                if outcome.protocol_violation {
                    last_protocol_violation = true;
                }

                if step_state_records_incident(step_state) {
                    // Preserve the first failure — subsequent handler failures
                    // should not overwrite the original root cause.
                    if last_failure.is_none() {
                        last_failure = Some(FailureInfo {
                            error_code: outcome.error_code.clone().unwrap_or_default(),
                            error_message: outcome.error_message.clone().unwrap_or_default(),
                            agent: (!execution.agent_cli.trim().is_empty())
                                .then(|| normalize_agent_label(&execution.agent_cli)),
                            model: resolved_model_name(host, &execution),
                        });
                    }
                    final_state = step_state;
                } else if step_state == JobRunState::Success && final_state != JobRunState::Success
                {
                    // A successful step after a failure means recovery (e.g.
                    // on_failure fallback fixed the issue). Reset final_state
                    // so the pipeline is not marked as failed.
                    final_state = JobRunState::Success;
                }

                // Check for loop_exit signal after each successful step, but
                // only when the job is actually looping (max_iterations > 1).
                // Single-pass pipelines must not exit early on loop_exit — the
                // signal is meant for nested looping jobs like job_review_loop.
                if max_iterations > 1
                    && step_state == JobRunState::Success
                    && check_loop_exit(host, &current_input)
                {
                    break 'outer;
                }
            }

            // If any step failed in this iteration, stop looping.
            if final_state != JobRunState::Success {
                break;
            }
        }

        let finished_at = Utc::now();
        let duration_ms = (total_duration_ms > 0).then_some(total_duration_ms);

        let changed = host.finalize_job_run(&run.run_id, final_state, finished_at, duration_ms)?;
        if !changed {
            return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
        }
        info!(state = %final_state, duration_ms = ?duration_ms, "job run completed");
        if final_state != JobRunState::Success {
            release_task_locks_for_job_input(host, &input)?;
        }
        host.record_event(OrbitEvent::JobRunCompleted {
            job_id: job.job_id.clone(),
            run_id: run.run_id.clone(),
            state: final_state.to_string(),
        })?;

        if create_failure_task
            && !matches!(final_state, JobRunState::Success | JobRunState::Cancelled)
            && let Some(ref failure) = last_failure
        {
            let _ = host.maybe_create_failure_task(
                &job.job_id,
                &run.run_id,
                &failure.error_code,
                &failure.error_message,
                failure.agent.as_deref(),
                failure.model.as_deref(),
            );
        }

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
            error!(error = %err, "job run failed before completion");
            if let Some(active_run) = host.get_job_run(&run.run_id)?
                && matches!(
                    active_run.state,
                    JobRunState::Pending | JobRunState::Running
                )
            {
                let (step_index, step) = &failure_step;
                finalize_failed_started_run(
                    host,
                    data_root,
                    &job,
                    &run,
                    *step_index,
                    step,
                    started_at,
                    &err,
                )?;
                if create_failure_task {
                    let agent = failure_step.1.agent_cli.trim();
                    let _ = host.maybe_create_failure_task(
                        &job.job_id,
                        &run.run_id,
                        ACTIVITY_EXECUTION_FAILED,
                        &err.to_string(),
                        (!agent.is_empty())
                            .then(|| normalize_agent_label(agent))
                            .as_deref(),
                        failure_step.1.model.as_deref(),
                    );
                }
            }
            release_task_locks_for_job_input(host, &input)?;
            Err(err)
        }
    }
}

pub fn recover_stale_active_run_for_job<H: JobRunHost + RuntimeHost>(
    host: &H,
    data_root: &Path,
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
            && owner_process_missing_or_reused(&active_run)
        {
            let error_message = abandoned_run_message(&active_run, pid);
            eprintln!(
                "orbit: abandoning stale run '{}' ({})",
                active_run.run_id, error_message
            );
            let duration_ms = active_run
                .started_at
                .map(|started| now.signed_duration_since(started).num_milliseconds().max(0) as u64);
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
                        error_message: Some(error_message.clone()),
                    },
                );
                append_failed_step_friction_without_execution(
                    data_root,
                    &active_run.run_id,
                    &first_step.target_id,
                    FrictionContext::default(),
                    Some(1),
                    &error_message,
                    now,
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
            append_failed_step_friction_without_execution(
                data_root,
                &active_run.run_id,
                &first_step.target_id,
                FrictionContext::default(),
                Some(1),
                &message,
                now,
            );
        }

        let changed =
            host.finalize_job_run(&active_run.run_id, JobRunState::Failed, now, duration_ms)?;
        if !changed {
            return Err(OrbitError::JobRunNotFound(active_run.run_id.clone()));
        }
        let empty_input = Value::Null;
        if let Some(task_id) = extract_task_id(active_run.input.as_ref().unwrap_or(&empty_input)) {
            let _ = host.release_file_locks(task_id);
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

#[cfg(unix)]
fn owner_process_missing_or_reused(run: &JobRun) -> bool {
    let Some(pid) = run.pid else {
        return false;
    };
    if !pid_is_alive(pid) {
        return true;
    }
    let Some(expected) = run.pid_start_time.as_deref() else {
        return false;
    };
    process_start_time_token(pid)
        .as_deref()
        .is_some_and(|actual| actual != expected)
}

#[cfg(not(unix))]
fn owner_process_missing_or_reused(_run: &JobRun) -> bool {
    false
}

#[cfg(unix)]
fn abandoned_run_message(run: &JobRun, pid: u32) -> String {
    if let Some(expected) = run.pid_start_time.as_deref()
        && process_start_time_token(pid)
            .as_deref()
            .is_some_and(|actual| actual != expected)
    {
        return format!(
            "run abandoned: owner pid {pid} no longer matches the recorded process identity"
        );
    }
    format!("run abandoned: owner pid {pid} is no longer alive")
}

#[cfg(not(unix))]
fn abandoned_run_message(_run: &JobRun, pid: u32) -> String {
    format!("run abandoned: owner pid {pid} is no longer alive")
}

#[cfg(unix)]
fn process_start_time_token(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!token.is_empty()).then_some(token)
}

#[allow(clippy::too_many_arguments)]
fn finalize_failed_started_run<H: JobRunHost + RuntimeHost>(
    host: &H,
    data_root: &Path,
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
    append_failed_step_friction_without_execution(
        data_root,
        &run.run_id,
        &step.target_id,
        FrictionContext::default(),
        Some(1),
        &message,
        finished_at,
    );

    let changed =
        host.finalize_job_run(&run.run_id, JobRunState::Failed, finished_at, duration_ms)?;
    if !changed {
        return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
    }
    release_task_locks_for_job_input(host, run.input.as_ref().unwrap_or(&Value::Null))?;
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

fn log_step_completion(
    step_index: usize,
    iteration: u32,
    step: &JobStep,
    state: JobRunState,
    duration_ms: Option<u64>,
    error_code: Option<&str>,
    error_message: Option<&str>,
) {
    if step_state_records_incident(state) {
        info!(
            step_index,
            iteration,
            target_id = %step.target_id,
            target_type = %step.target_type,
            state = %state,
            duration_ms = ?duration_ms,
            error_code = error_code.unwrap_or(""),
            error_message = error_message.unwrap_or(""),
            "step failed"
        );
    } else {
        info!(
            step_index,
            iteration,
            target_id = %step.target_id,
            target_type = %step.target_type,
            state = %state,
            duration_ms = ?duration_ms,
            "step completed"
        );
    }
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

/// Execute a nested job step by loading the referenced job and running it.
/// Nested jobs do not create their own failure tasks — the outermost pipeline
/// is responsible for creating a single failure task.
fn execute_job_step<H: EngineHost>(
    host: &H,
    data_root: &Path,
    job_id: &str,
    input: &Value,
    debug: bool,
) -> Result<JobRunResult, OrbitError> {
    let sub_job = host
        .get_job(job_id)?
        .ok_or_else(|| OrbitError::JobValidation(format!("nested job '{}' not found", job_id)))?;
    let _ = recover_stale_active_run_for_job(host, data_root, &sub_job, Utc::now())?;
    let active_runs = host.list_pending_or_running_job_runs(&sub_job.job_id)?;
    if active_runs.len() as u32 >= sub_job.max_active_runs {
        let latest_active_run = active_runs.first().ok_or_else(|| {
            OrbitError::JobValidation(format!(
                "job '{}' has no active runs despite reaching max_active_runs={}",
                sub_job.job_id, sub_job.max_active_runs
            ))
        })?;
        return Err(OrbitError::JobValidation(format!(
            "job '{}' already has {} active run(s), reaching max_active_runs={} (latest active run '{}' in state '{}')",
            sub_job.job_id,
            active_runs.len(),
            sub_job.max_active_runs,
            latest_active_run.run_id,
            latest_active_run.state,
        )));
    }
    host.record_event(OrbitEvent::JobTriggered {
        job_id: sub_job.job_id.clone(),
    })?;
    execute_activity_with_retries(
        host,
        data_root,
        sub_job,
        ActivityExecutionRequest {
            scheduled_at: Utc::now(),
            initial_run: None,
            input: input.clone(),
            debug,
            create_failure_task: false,
            skip_to_step: 0,
            replayed_steps: &[],
        },
    )
}

/// Returns `true` if the accumulated input contains `"loop_exit": true`.
fn check_loop_exit<H: crate::context::TaskHost + ?Sized>(host: &H, input: &Value) -> bool {
    // Primary: check for explicit loop_exit signal in piped input.
    let explicit = input
        .as_object()
        .and_then(|map| map.get("loop_exit"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if explicit {
        return true;
    }

    // Fallback: if the agent persisted pr_status to the task but crashed before
    // returning structured output (with loop_exit), check the task directly.
    if let Some(task_id) = extract_task_id(input)
        && let Ok(task) = host.get_task(task_id)
        && let Some(ref pr_status) = task.pr_status
    {
        let normalized = crate::executor::automation::review::normalize_review_decision(pr_status);
        if normalized == "APPROVED" {
            return true;
        }
    }

    false
}

fn should_run_step(condition: StepCondition, previous_step_state: Option<JobRunState>) -> bool {
    match condition {
        StepCondition::Always => true,
        StepCondition::OnSuccess => {
            previous_step_state.is_none_or(|state| matches!(state, JobRunState::Success))
        }
        StepCondition::OnFailure => previous_step_state.is_some_and(step_state_records_failure),
        StepCondition::OnTimeout => {
            previous_step_state.is_some_and(|state| matches!(state, JobRunState::Timeout))
        }
    }
}

fn step_state_records_failure(state: JobRunState) -> bool {
    matches!(
        state,
        JobRunState::Failed | JobRunState::Timeout | JobRunState::Cancelled
    )
}

fn step_state_records_incident(state: JobRunState) -> bool {
    matches!(state, JobRunState::Failed | JobRunState::Timeout)
}

fn run_was_cancelled<H: JobRunHost>(host: &H, run_id: &str) -> Result<bool, OrbitError> {
    Ok(host
        .get_job_run(run_id)?
        .is_some_and(|run| run.state == JobRunState::Cancelled))
}

/// When a step's `agent_cli` is empty, try to resolve it from the task's
/// `agent` and `model` fields so the original implementer handles the step
/// (e.g. in a review-loop where the fix should go back to the same agent).
fn resolve_step_agent_from_task<H: EngineHost>(
    host: &H,
    step: &JobStep,
    input: &Value,
) -> Option<JobStep> {
    if !step.agent_cli.trim().is_empty() {
        return None;
    }
    let task_id = extract_task_id(input)?;
    let task = host.get_task(task_id).ok()?;
    let agent = task
        .actor_identity
        .agent_name()
        .filter(|a| !a.trim().is_empty())?;
    let mut resolved = step.clone();
    resolved.agent_cli = agent.to_string();
    if resolved.model.is_none() {
        resolved.model = task.actor_identity.agent_model().map(ToOwned::to_owned);
    }
    Some(resolved)
}

fn record_task_agent_context<H: EngineHost>(
    host: &H,
    execution: &crate::context::ExecutionContext,
) -> Result<(), OrbitError> {
    if execution.agent_cli.trim().is_empty() {
        return Ok(());
    }
    let Some(task_id) = extract_task_id(&execution.input) else {
        return Ok(());
    };

    host.apply_task_automation_update(
        task_id,
        crate::context::TaskAutomationUpdate {
            agent: Some(normalize_agent_label(&execution.agent_cli)),
            model: resolved_model_name(host, execution),
            ..Default::default()
        },
    )
}

fn resolved_model_name<H: EngineHost>(
    host: &H,
    execution: &crate::context::ExecutionContext,
) -> Option<String> {
    let config = host
        .agent_config_for(&execution.agent_cli, execution.model.as_deref())
        .ok()?;
    let model_from_config = config.model.clone();
    let agent = Agent::new(&config).ok();
    agent
        .and_then(|agent| agent.model_name().map(ToOwned::to_owned))
        .or(model_from_config)
}

/// Captures information about the first step failure in a pipeline run,
/// including agent attribution for the failure task.
struct FailureInfo {
    error_code: String,
    error_message: String,
    agent: Option<String>,
    model: Option<String>,
}

#[derive(Default)]
struct FrictionContext {
    input: Option<Value>,
    command: Option<String>,
    agent: Option<String>,
    model: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn append_failed_step_friction<H: EngineHost>(
    data_root: &Path,
    host: &H,
    run_id: &str,
    step_id: &str,
    execution: &crate::context::ExecutionContext,
    exit_code: Option<i32>,
    stderr: &str,
    ts: DateTime<Utc>,
) {
    append_failed_step_friction_without_execution(
        data_root,
        run_id,
        step_id,
        FrictionContext {
            input: Some(execution.input.clone()),
            command: Some(command_label(execution)),
            agent: (!execution.agent_cli.trim().is_empty())
                .then(|| normalize_agent_label(&execution.agent_cli)),
            model: resolved_model_name(host, execution),
        },
        exit_code,
        stderr,
        ts,
    );
}

fn append_failed_step_friction_without_execution(
    data_root: &Path,
    run_id: &str,
    step_id: &str,
    context: FrictionContext,
    exit_code: Option<i32>,
    stderr: &str,
    ts: DateTime<Utc>,
) {
    let input = context
        .input
        .unwrap_or_else(|| Value::Object(Default::default()));
    let actor_identity =
        ActorIdentity::from_legacy(context.agent.as_deref(), context.model.as_deref());
    let entry = FrictionEntry {
        ts,
        job_run: run_id.to_string(),
        step: step_id.to_string(),
        task_id: extract_task_id(&input).map(ToOwned::to_owned),
        command: context.command.unwrap_or_else(|| step_id.to_string()),
        input: serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string()),
        exit_code,
        stderr: stderr.to_string(),
        actor_identity,
    };
    if let Err(error) = append_friction_entry(data_root, &entry) {
        eprintln!("orbit: failed to append friction log entry: {error}");
    }
}

fn command_label(execution: &crate::context::ExecutionContext) -> String {
    let config = &execution.activity.spec_config;
    match execution.activity.spec_type.as_str() {
        "automation" => config
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or(execution.activity.id.as_str())
            .to_string(),
        "cli_command" => config
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or(execution.activity.id.as_str())
            .to_string(),
        "agent_invoke" => normalize_agent_label(&execution.agent_cli),
        _ => execution.activity.id.to_string(),
    }
}

fn extract_task_id(input: &Value) -> Option<&str> {
    input
        .as_object()
        .and_then(|map| map.get("task_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn normalize_agent_label(agent_cli: &str) -> String {
    std::path::Path::new(agent_cli)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(agent_cli)
        .to_ascii_lowercase()
}

fn release_task_locks_for_job_input<H: RuntimeHost>(
    host: &H,
    input: &Value,
) -> Result<(), OrbitError> {
    if let Some(task_id) = extract_task_id(input) {
        let _ = host.release_file_locks(task_id)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_step_metrics<H: EngineHost>(
    data_root: &Path,
    host: &H,
    run_id: &str,
    step_id: &str,
    execution: &crate::context::ExecutionContext,
    duration_ms: Option<u64>,
    retry_count: u32,
    ts: DateTime<Utc>,
) {
    let agent = (!execution.agent_cli.trim().is_empty())
        .then(|| normalize_agent_label(&execution.agent_cli));
    let model = resolved_model_name(host, execution);
    let task_id = extract_task_id(&execution.input).map(ToOwned::to_owned);

    let actor_identity = ActorIdentity::from_legacy(agent.as_deref(), model.as_deref());
    let entry = MetricsEntry {
        ts,
        job_run: run_id.to_string(),
        step: step_id.to_string(),
        task_id,
        actor_identity,
        tool_invocations: 0, // Not yet tracked at the engine level
        token_usage: None,   // Not yet tracked at the engine level
        step_duration_ms: duration_ms,
        retry_count,
    };
    if let Err(error) = append_metrics_entry(data_root, &entry) {
        eprintln!("orbit: failed to append metrics log entry: {error}");
    }
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
