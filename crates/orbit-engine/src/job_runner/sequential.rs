use chrono::{DateTime, Utc};
use orbit_store::JobRunStepParams;
use orbit_types::{Job, JobRun, JobRunState, JobTargetType, OrbitError, OrbitEvent, PipelineState};
use serde_json::Value;
use std::path::Path;
use tracing::{error, info, info_span, warn};

use crate::activity_runner::{build_execution_context_for_step, execute_with_retry};
use crate::context::{
    ACTIVITY_EXECUTION_FAILED, EngineHost, ExecutorLookupHost, INPUT_VALIDATION_FAILED,
    JobRunResult, blocked_workflow_failure_update,
};

use super::execution::execute_job_step;
use super::friction::{append_failed_step_friction, append_step_metrics};
use super::helpers::{
    build_knowledge_run_metrics, build_step_input, check_loop_exit, extract_batch_id,
    extract_task_id, log_step_completion, merge_job_input, normalize_agent_label,
    prepare_implement_change_metrics, record_task_agent_context, resolve_step_agent,
    resolve_step_agent_from_input, resolved_model_name, run_was_cancelled, should_run_step,
    step_state_records_incident,
};
use super::pipeline_recovery::{
    apply_output_map, apply_pipeline_patch, build_steps_template_outputs,
    final_state_from_pipeline_state, pipeline_patch_for_job_step, step_recovery_key,
};
use super::stale_recovery::finalize_failed_started_run;

pub(crate) struct ActivityExecutionRequest<'a> {
    pub scheduled_at: DateTime<Utc>,
    pub initial_run: Option<JobRun>,
    pub input: Value,
    pub initial_pipeline_state: Option<PipelineState>,
    pub debug: bool,
    // When `true`, a failure task is created on pipeline failure.
    // Nested (sub-job) runs pass `false` so only the outermost pipeline
    // creates a single failure task.
    pub create_failure_task: bool,
    // When > 0, steps before this index are written as Skipped records with
    // replayed data from the source run. Execution starts from this index.
    pub skip_to_step: usize,
    // Source run steps used to replay data when `skip_to_step > 0`.
    pub replayed_steps: &'a [orbit_types::JobRunStep],
    // When `true`, skipped steps before `skip_to_step` are treated as already
    // completed in this same run and must not be rewritten as `Skipped`.
    pub preserve_existing_step_records: bool,
}

fn apply_loop_outcome_metadata(
    current_input: &mut Value,
    final_state: JobRunState,
    max_iterations: u32,
    loop_iterations_completed: u32,
    loop_exited: bool,
) -> (JobRunState, bool) {
    let exhausted = !loop_exited
        && final_state == JobRunState::Success
        && loop_iterations_completed == max_iterations;
    if let Value::Object(input_map) = current_input {
        input_map.insert(
            "fix_loop_iterations".to_string(),
            Value::from(loop_iterations_completed),
        );
        input_map.insert("fix_loop_exhausted".to_string(), Value::from(exhausted));
    }
    (
        if exhausted {
            JobRunState::Failed
        } else {
            final_state
        },
        exhausted,
    )
}

pub(crate) fn execute_activity_with_retries<H: EngineHost + ExecutorLookupHost>(
    host: &H,
    data_root: &Path,
    job: Job,
    request: ActivityExecutionRequest<'_>,
) -> Result<JobRunResult, OrbitError> {
    let ActivityExecutionRequest {
        scheduled_at,
        initial_run,
        input,
        initial_pipeline_state,
        debug,
        create_failure_task,
        skip_to_step,
        replayed_steps,
        preserve_existing_step_records,
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

    let claimed_from_pending = run.state == JobRunState::Pending;
    let started_at = if claimed_from_pending {
        Utc::now()
    } else {
        run.started_at.unwrap_or_else(Utc::now)
    };
    let changed = if claimed_from_pending {
        host.mark_job_run_running(&run.run_id, started_at, std::process::id())?
    } else {
        host.take_over_running_job_run(
            &run.run_id,
            run.pid,
            run.pid_start_time.clone(),
            started_at,
            std::process::id(),
        )?
    };
    if !changed {
        return Err(OrbitError::Execution(format!(
            "job run '{}' is no longer claimable",
            run.run_id
        )));
    }
    if claimed_from_pending {
        host.record_event(OrbitEvent::JobRunStarted {
            job_id: job.job_id.clone(),
            run_id: run.run_id.clone(),
            attempt: run.attempt,
        })?;
    }
    run.state = JobRunState::Running;
    run.started_at = Some(started_at);
    run.pid = Some(std::process::id());

    let default_failure_step =
        job.steps.first().cloned().ok_or_else(|| {
            OrbitError::JobValidation("job must have at least one step".to_string())
        })?;
    let mut failure_step = (0usize, default_failure_step);

    let execution_result: Result<JobRunResult, OrbitError> = (|| {
        let mut total_duration_ms: u64 = 0;
        let mut last_protocol_violation = false;
        let mut current_input = if let Some(seed) = initial_pipeline_state.as_ref() {
            seed.pipeline.clone()
        } else {
            merge_job_input(job.default_input.as_ref(), input.clone())?
        };
        // Inject run_id so all steps can reference it (e.g. as batch_id for
        // parallel task pipelines).
        if let Value::Object(ref mut map) = current_input {
            map.insert("run_id".to_string(), Value::String(run.run_id.clone()));
        }
        let seeded_from_state = initial_pipeline_state.is_some();
        let mut steps_outputs = initial_pipeline_state
            .as_ref()
            .map(|state| build_steps_template_outputs(&job, state, state.next_step_index as usize))
            .unwrap_or_default();

        // Create and persist initial pipeline state.
        let mut pipeline_state = initial_pipeline_state.unwrap_or_else(|| {
            PipelineState::new(
                run.run_id.clone(),
                job.job_id.clone(),
                current_input.clone(),
            )
        });
        pipeline_state.run_id = run.run_id.clone();
        pipeline_state.job_id = job.job_id.clone();
        if !seeded_from_state {
            pipeline_state.initial_input = current_input.clone();
        }
        pipeline_state.sync_pipeline(current_input.clone());
        if let Err(e) = host.write_run_state(&run.run_id, &pipeline_state) {
            warn!(error = %e, "failed to write initial pipeline state");
        }

        let mut final_state = if preserve_existing_step_records {
            final_state_from_pipeline_state(&pipeline_state)
        } else {
            JobRunState::Success
        };

        // DAG execution: if any step has `upstream`, use the DAG scheduler
        // instead of the sequential loop. DAG is incompatible with looping.
        if super::dag::is_dag_job(&job.steps) {
            super::dag::validate_dag(&job.steps)?;
            if job.max_iterations > 1 {
                return Err(OrbitError::JobValidation(
                    "DAG jobs (steps with 'upstream') cannot have max_iterations > 1".to_string(),
                ));
            }
            return super::dag::execute_dag(
                host,
                &job,
                &run,
                current_input,
                debug,
                create_failure_task,
            );
        }

        let mut last_failure: Option<FailureInfo> = None;
        let num_steps = job.steps.len();
        let max_iterations = job.max_iterations.max(1);
        let looping_job = max_iterations > 1;
        let mut loop_iterations_completed = 0u32;
        let mut loop_exited = false;
        let mut loop_exhausted = false;

        'outer: for iteration in 0..max_iterations {
            if looping_job && check_loop_exit(host, &current_input) {
                loop_exited = true;
                break;
            }
            let mut previous_step_state = if iteration == 0 {
                pipeline_state.previous_step_state
            } else {
                None
            };

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

                    if let Some(src) = source_step
                        && src.state != JobRunState::Skipped
                    {
                        previous_step_state = Some(src.state);
                    } else if preserve_existing_step_records {
                        previous_step_state = pipeline_state
                            .previous_step_state_before((global_step_index + 1) as u32);
                    }

                    if !seeded_from_state && !preserve_existing_step_records {
                        pipeline_state.record_step(
                            global_step_index as u32,
                            JobRunState::Skipped,
                            None,
                            None,
                        );
                        if let Err(e) = host.write_run_state(&run.run_id, &pipeline_state) {
                            warn!(error = %e, "failed to persist replayed retry state");
                        }
                    }
                    if !preserve_existing_step_records {
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
                    }
                    continue;
                }

                failure_step = (global_step_index, step.clone());

                let should_run = match &step.condition {
                    orbit_types::StepCondition::Expr(_) => {
                        let cond_ctx = crate::template::TemplateContext {
                            input: current_input.clone(),
                            steps: steps_outputs.clone(),
                            ..Default::default()
                        };
                        super::condition::evaluate_condition(&step.condition, &cond_ctx, |c| {
                            should_run_step(c, previous_step_state)
                        })?
                    }
                    _ => should_run_step(&step.condition, previous_step_state),
                };
                if !should_run {
                    let skipped_at = Utc::now();
                    pipeline_state.record_step(
                        global_step_index as u32,
                        JobRunState::Skipped,
                        None,
                        None,
                    );
                    if let Err(e) = host.write_run_state(&run.run_id, &pipeline_state) {
                        warn!(error = %e, "failed to persist skipped step state");
                    }
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
                    let step_input = build_step_input(step, &current_input)?;
                    let step_started = Utc::now();
                    let sub_result =
                        execute_job_step(host, data_root, &step.target_id, &step_input, debug);
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
                    let pipeline_patch = if step_state == JobRunState::Success {
                        pipeline_patch_for_job_step(
                            sub_result
                                .as_ref()
                                .ok()
                                .and_then(|result| result.output.as_ref()),
                        )
                    } else {
                        None
                    };
                    if let Some(patch) = pipeline_patch.as_ref() {
                        apply_pipeline_patch(&mut current_input, patch);
                    }
                    pipeline_state.record_step(
                        global_step_index as u32,
                        step_state,
                        None,
                        pipeline_patch,
                    );
                    if let Err(e) = host.write_run_state(&run.run_id, &pipeline_state) {
                        warn!(error = %e, "failed to persist pipeline state after sub-job");
                    }
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

                    if step_state == JobRunState::Cancelled {
                        final_state = JobRunState::Cancelled;
                        break 'outer;
                    }

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
                // If the step's agent_cli is empty, resolve it via the
                // precedence chain: agent_cli_from_input (job input) first,
                // then task actor identity. See `resolve_step_agent` docs.
                let step_input = build_step_input(step, &current_input)?;
                let resolved_step = resolve_step_agent(host, step, &step_input);
                let resolved_from_input = resolve_step_agent_from_input(step, &step_input);
                let effective_step = resolved_step.as_ref().unwrap_or(step);
                let execution = build_execution_context_for_step(
                    host,
                    &job,
                    effective_step,
                    step_input,
                    debug,
                    steps_outputs.clone(),
                    Some(&run.run_id),
                    Some(global_step_index as u32),
                )?;
                // Record agent context for explicit steps and input-driven
                // assignments, but not task-actor fallback.
                if !step.agent_cli.trim().is_empty() || resolved_from_input.is_some() {
                    record_task_agent_context(host, &execution)?;
                }
                let prepared_knowledge_metrics =
                    prepare_implement_change_metrics(host, &execution)?;
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
                previous_step_state = Some(step_state);

                // Pipe this step's output fields into the next step's input.
                let step_key = step_recovery_key(step);
                let step_output = match execution.state_dir.as_deref() {
                    Some(state_dir) => orbit_store::state_io::read_step_output(
                        state_dir,
                        global_step_index as u32,
                    )?,
                    None => None,
                };
                {
                    use super::pipeline_recovery::wrap_step_entry;
                    let entry = wrap_step_entry(
                        Some(step_state),
                        outcome.exit_code,
                        outcome.duration_ms,
                        step_output.as_ref(),
                    );
                    steps_outputs.insert(step_key, entry);
                }
                let pipeline_patch = if step_state == JobRunState::Success {
                    step_output
                        .clone()
                        .map(|output| apply_output_map(output, &step.output_map))
                } else {
                    None
                };
                if let Some(patch) = pipeline_patch.as_ref() {
                    apply_pipeline_patch(&mut current_input, patch);
                }
                pipeline_state.record_step(
                    global_step_index as u32,
                    step_state,
                    step_output,
                    pipeline_patch,
                );
                if let Err(e) = host.write_run_state(&run.run_id, &pipeline_state) {
                    warn!(error = %e, "failed to persist pipeline state");
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

                if execution.activity.spec_type == "agent_invoke" {
                    host.persist_invocation_trace(
                        &run.run_id,
                        &execution,
                        &outcome.invocation_trace,
                    )?;
                }

                if step_state == JobRunState::Success
                    && let Some(prepared) = prepared_knowledge_metrics.as_ref()
                {
                    let knowledge_metrics =
                        build_knowledge_run_metrics(prepared, &outcome.invocation_trace)?;
                    let changed =
                        host.record_job_run_knowledge_metrics(&run.run_id, knowledge_metrics)?;
                    if !changed {
                        return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
                    }
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
                    outcome.invocation_trace.tool_calls.len() as u32,
                    Some(outcome.invocation_trace.usage.prompt_response_total()),
                    outcome.retry_count,
                    step_finished,
                );

                if outcome.protocol_violation {
                    last_protocol_violation = true;
                }

                if step_state == JobRunState::Cancelled {
                    final_state = JobRunState::Cancelled;
                    break 'outer;
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
                    loop_iterations_completed = iteration + 1;
                    loop_exited = true;
                    break 'outer;
                }
            }

            if looping_job && final_state == JobRunState::Success {
                loop_iterations_completed = iteration + 1;
                pipeline_state.set_iteration(loop_iterations_completed);
                if let Err(e) = host.write_run_state(&run.run_id, &pipeline_state) {
                    warn!(error = %e, "failed to persist pipeline state at loop boundary");
                }
            }

            // If any step failed in this iteration, stop looping.
            if final_state != JobRunState::Success {
                break;
            }
        }

        if looping_job {
            let (adjusted_state, exhausted) = apply_loop_outcome_metadata(
                &mut current_input,
                final_state,
                max_iterations,
                loop_iterations_completed,
                loop_exited,
            );
            final_state = adjusted_state;
            loop_exhausted = exhausted;
        }

        pipeline_state.sync_pipeline(current_input.clone());
        if let Err(e) = host.write_run_state(&run.run_id, &pipeline_state) {
            warn!(error = %e, "failed to persist final pipeline state");
        }

        let finished_at = Utc::now();
        let duration_ms = (total_duration_ms > 0).then_some(total_duration_ms);

        let changed = host.finalize_job_run(&run.run_id, final_state, finished_at, duration_ms)?;
        if !changed {
            return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
        }
        info!(state = %final_state, duration_ms = ?duration_ms, "job run completed");
        host.record_event(OrbitEvent::JobRunCompleted {
            job_id: job.job_id.clone(),
            run_id: run.run_id.clone(),
            state: final_state.to_string(),
        })?;

        if !matches!(final_state, JobRunState::Success | JobRunState::Cancelled)
            && let Some(task_id) = extract_task_id(&current_input)
        {
            let loop_failure_message = format!("loop exhausted after {max_iterations} iterations");
            let (error_code, error_message) = if loop_exhausted {
                (Some("LOOP_EXHAUSTED"), Some(loop_failure_message.as_str()))
            } else {
                (
                    last_failure
                        .as_ref()
                        .map(|failure| failure.error_code.as_str()),
                    last_failure
                        .as_ref()
                        .map(|failure| failure.error_message.as_str()),
                )
            };
            let _ = host.apply_task_automation_update(
                task_id,
                blocked_workflow_failure_update(
                    &job.job_id,
                    &run.run_id,
                    error_code,
                    error_message,
                ),
            );
        }

        if create_failure_task
            && !matches!(final_state, JobRunState::Success | JobRunState::Cancelled)
            && let Some(ref failure) = last_failure
            && failure.error_code != INPUT_VALIDATION_FAILED
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
            output: Some(current_input),
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
                let error_code = if matches!(err, OrbitError::InvalidInput(_)) {
                    INPUT_VALIDATION_FAILED
                } else {
                    ACTIVITY_EXECUTION_FAILED
                };
                if create_failure_task && error_code != INPUT_VALIDATION_FAILED {
                    let agent = failure_step.1.agent_cli.trim();
                    let _ = host.maybe_create_failure_task(
                        &job.job_id,
                        &run.run_id,
                        error_code,
                        &err.to_string(),
                        (!agent.is_empty())
                            .then(|| normalize_agent_label(agent))
                            .as_deref(),
                        failure_step.1.model.as_deref(),
                    );
                }
            }
            if let Some(task_id) = extract_task_id(&input)
                .or_else(|| job.default_input.as_ref().and_then(extract_task_id))
            {
                let error_code = if matches!(err, OrbitError::InvalidInput(_)) {
                    INPUT_VALIDATION_FAILED
                } else {
                    ACTIVITY_EXECUTION_FAILED
                };
                let _ = host.apply_task_automation_update(
                    task_id,
                    blocked_workflow_failure_update(
                        &job.job_id,
                        &run.run_id,
                        Some(error_code),
                        Some(&err.to_string()),
                    ),
                );
            } else if let Some(batch_id) = extract_batch_id(&input)
                .or_else(|| job.default_input.as_ref().and_then(extract_batch_id))
            {
                if let Ok(tasks) = host.list_tasks_filtered(None, None, None, Some(batch_id)) {
                    let error_code = if matches!(err, OrbitError::InvalidInput(_)) {
                        INPUT_VALIDATION_FAILED
                    } else {
                        ACTIVITY_EXECUTION_FAILED
                    };
                    for task in tasks {
                        let _ = host.apply_task_automation_update(
                            &task.id,
                            blocked_workflow_failure_update(
                                &job.job_id,
                                &run.run_id,
                                Some(error_code),
                                Some(&err.to_string()),
                            ),
                        );
                    }
                }
            }
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
