use chrono::{DateTime, Utc};
use orbit_store::JobRunStepParams;
use orbit_types::{Job, JobRun, JobRunState, JobTargetType, OrbitError, OrbitEvent};
use serde_json::Value;
use std::path::Path;
use tracing::{error, info, info_span, warn};

use crate::activity_runner::{build_execution_context_for_step, execute_with_retry};
use crate::context::{
    ACTIVITY_EXECUTION_FAILED, EngineHost, JobRunResult, step_output_for_following_input,
};

use super::friction::{append_failed_step_friction, append_step_metrics};
use super::helpers::{
    check_loop_exit, log_step_completion, merge_job_input, normalize_agent_label,
    record_task_agent_context, release_task_locks_for_job_input, resolve_step_agent_from_task,
    resolved_model_name, run_was_cancelled, should_run_step, step_state_records_incident,
};
use super::stale_recovery::{finalize_failed_started_run, recover_stale_active_run_for_job};

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

/// Captures information about the first step failure in a pipeline run,
/// including agent attribution for the failure task.
struct FailureInfo {
    error_code: String,
    error_message: String,
    agent: Option<String>,
    model: Option<String>,
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
