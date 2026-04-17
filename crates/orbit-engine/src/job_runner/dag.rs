//! DAG execution support for non-looping jobs.
//!
//! When any step in a job declares `upstream` dependencies, the job uses DAG
//! scheduling instead of sequential execution. Steps with satisfied dependencies
//! run in parallel via `std::thread::scope`.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use chrono::Utc;
use orbit_store::JobRunStepParams;
use orbit_types::{Job, JobRun, JobRunState, JobStep, OrbitError, OrbitEvent, PipelineState};
use serde_json::Value;

use crate::activity_runner::{build_execution_context_for_step, execute_with_retry};
use crate::context::{EngineHost, ExecutorLookupHost, INPUT_VALIDATION_FAILED, JobRunResult};

use super::helpers::{build_step_input, resolve_step_agent, run_was_cancelled};
use super::pipeline_recovery::{apply_output_map, wrap_step_entry};

/// Returns `true` if any step in the job declares upstream dependencies.
pub(super) fn is_dag_job(steps: &[JobStep]) -> bool {
    steps.iter().any(|s| !s.upstream.is_empty())
}

/// Validate DAG constraints at job load time.
///
/// Rules:
/// - If any step has `upstream`, ALL steps must have `id`.
/// - No duplicate `id` values.
/// - All upstream references must resolve to existing step ids.
/// - No cycles (detected via Kahn's algorithm).
pub(super) fn validate_dag(steps: &[JobStep]) -> Result<(), OrbitError> {
    if !is_dag_job(steps) {
        return Ok(());
    }

    // All steps must have id when any step has upstream.
    for (i, step) in steps.iter().enumerate() {
        if step.id.is_none() {
            return Err(OrbitError::JobValidation(format!(
                "step {} ({}) must have an 'id' when any step uses 'upstream'",
                i, step.target_id
            )));
        }
    }

    // Collect ids and check for duplicates.
    let mut id_to_index: HashMap<&str, usize> = HashMap::new();
    for (i, step) in steps.iter().enumerate() {
        let id = step.id.as_deref().unwrap(); // safe: checked above
        if let Some(prev) = id_to_index.insert(id, i) {
            return Err(OrbitError::JobValidation(format!(
                "duplicate step id '{}' at indices {} and {}",
                id, prev, i
            )));
        }
    }

    // All upstream references must resolve.
    for step in steps {
        let id = step.id.as_deref().unwrap();
        for upstream_ref in &step.upstream {
            if !id_to_index.contains_key(upstream_ref.as_str()) {
                return Err(OrbitError::JobValidation(format!(
                    "step '{}' references unknown upstream '{}'",
                    id, upstream_ref
                )));
            }
        }
    }

    // Cycle detection via Kahn's algorithm.
    let n = steps.len();
    let mut in_degree = vec![0u32; n];
    let mut adjacency: Vec<Vec<usize>> = vec![vec![]; n];

    for (i, step) in steps.iter().enumerate() {
        for upstream_ref in &step.upstream {
            let j = id_to_index[upstream_ref.as_str()];
            adjacency[j].push(i);
            in_degree[i] += 1;
        }
    }

    let mut queue: VecDeque<usize> = VecDeque::new();
    for (i, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(i);
        }
    }

    let mut visited = 0usize;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        for &neighbor in &adjacency[node] {
            in_degree[neighbor] -= 1;
            if in_degree[neighbor] == 0 {
                queue.push_back(neighbor);
            }
        }
    }

    if visited != n {
        return Err(OrbitError::JobValidation(
            "job step DAG contains a cycle".to_string(),
        ));
    }

    Ok(())
}

/// Return step indices in topological order.
#[allow(dead_code)]
pub(super) fn topological_order(steps: &[JobStep]) -> Result<Vec<usize>, OrbitError> {
    let n = steps.len();
    let id_to_index: HashMap<&str, usize> = steps
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.id.as_deref().map(|id| (id, i)))
        .collect();

    let mut in_degree = vec![0u32; n];
    let mut adjacency: Vec<Vec<usize>> = vec![vec![]; n];

    for (i, step) in steps.iter().enumerate() {
        for upstream_ref in &step.upstream {
            if let Some(&j) = id_to_index.get(upstream_ref.as_str()) {
                adjacency[j].push(i);
                in_degree[i] += 1;
            }
        }
    }

    let mut queue: VecDeque<usize> = VecDeque::new();
    for (i, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(i);
        }
    }

    let mut order = Vec::with_capacity(n);
    while let Some(node) = queue.pop_front() {
        order.push(node);
        for &neighbor in &adjacency[node] {
            in_degree[neighbor] -= 1;
            if in_degree[neighbor] == 0 {
                queue.push_back(neighbor);
            }
        }
    }

    if order.len() != n {
        return Err(OrbitError::JobValidation(
            "job step DAG contains a cycle".to_string(),
        ));
    }

    Ok(order)
}

/// Determine whether a DAG step should run based on its upstream results.
///
/// Unlike the sequential model (which uses `previous_step_state`), DAG steps
/// evaluate conditions against ALL their upstream dependencies.
pub(super) fn should_run_dag_step(
    condition: &orbit_types::StepCondition,
    upstream_ids: &[String],
    completed: &HashMap<String, orbit_types::JobRunState>,
) -> bool {
    use orbit_types::{JobRunState, StepCondition};

    if upstream_ids.is_empty() {
        return matches!(condition, StepCondition::Always | StepCondition::OnSuccess);
    }

    let upstream_states: Vec<JobRunState> = upstream_ids
        .iter()
        .filter_map(|id| completed.get(id).copied())
        .collect();

    // All upstream must have completed before we evaluate.
    if upstream_states.len() != upstream_ids.len() {
        return false;
    }

    match condition {
        StepCondition::Always => true,
        StepCondition::OnSuccess => upstream_states.iter().all(|s| *s == JobRunState::Success),
        StepCondition::OnFailure => upstream_states.iter().any(|s| {
            matches!(
                s,
                JobRunState::Failed | JobRunState::Timeout | JobRunState::Cancelled
            )
        }),
        StepCondition::OnTimeout => upstream_states.iter().any(|s| *s == JobRunState::Timeout),
        StepCondition::Expr(_) => {
            // Expression conditions are evaluated by the condition module with
            // full template context; this helper only handles keyword variants.
            true
        }
    }
}

/// DAG scheduler: executes steps in parallel based on `upstream` dependencies.
///
/// Steps with no upstream (root nodes) start immediately. Steps wait until all
/// their upstream dependencies complete. Multiple independent steps run in
/// parallel via `std::thread::scope`.
///
/// Input resolution: each step receives `job.default_input` merged with
/// `step.default_input` ONLY — no automatic upstream output merge. Steps use
/// `{{steps.<id>.<field>}}` templates for explicit cross-step references.
pub(super) fn execute_dag<H: EngineHost + ExecutorLookupHost + Sync>(
    host: &H,
    job: &Job,
    run: &JobRun,
    base_input: Value,
    debug: bool,
    _create_failure_task: bool,
) -> Result<JobRunResult, OrbitError> {
    let steps = &job.steps;
    let n = steps.len();

    let id_to_index: HashMap<&str, usize> = steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| step.id.as_deref().map(|id| (id, index)))
        .collect();

    let mut downstream: Vec<Vec<usize>> = vec![vec![]; n];
    let mut in_degree = vec![0u32; n];
    for (index, step) in steps.iter().enumerate() {
        for upstream_ref in &step.upstream {
            if let Some(&upstream_index) = id_to_index.get(upstream_ref.as_str()) {
                downstream[upstream_index].push(index);
                in_degree[index] += 1;
            }
        }
    }

    let restored_state = host.read_run_state(&run.run_id)?;
    let mut restored_completed: HashMap<String, (JobRunState, Option<Value>)> = HashMap::new();
    let mut restored_steps_outputs: HashMap<String, Value> = HashMap::new();
    let mut restored_remaining_in_degree = in_degree.clone();
    let mut restored_final_state = JobRunState::Success;

    if let Some(state) = restored_state.as_ref() {
        for (&step_index, &state_value) in &state.step_states {
            let Some(step) = steps.get(step_index as usize) else {
                continue;
            };
            let step_id = step.id.as_deref().unwrap_or(&step.target_id).to_string();
            let raw_output = state.step_outputs.get(&step_index).cloned();
            let recorded_step = run
                .steps
                .iter()
                .find(|entry| entry.step_index == step_index);
            restored_steps_outputs.insert(
                step_id.clone(),
                wrap_step_entry(
                    Some(state_value),
                    recorded_step.and_then(|entry| entry.exit_code),
                    recorded_step.and_then(|entry| entry.duration_ms),
                    raw_output.as_ref(),
                ),
            );
            restored_completed.insert(step_id, (state_value, raw_output));
            for &downstream_index in &downstream[step_index as usize] {
                restored_remaining_in_degree[downstream_index] =
                    restored_remaining_in_degree[downstream_index].saturating_sub(1);
            }
            if state_value == JobRunState::Cancelled {
                restored_final_state = JobRunState::Cancelled;
            } else if state_value != JobRunState::Success && state_value != JobRunState::Skipped {
                restored_final_state = state_value;
            }
        }
    }

    let completed: Mutex<HashMap<String, (JobRunState, Option<Value>)>> =
        Mutex::new(restored_completed);
    let steps_outputs: Mutex<HashMap<String, Value>> = Mutex::new(restored_steps_outputs);
    let remaining_in_degree: Mutex<Vec<u32>> = Mutex::new(restored_remaining_in_degree);
    let pipeline_state = Mutex::new(restored_state.unwrap_or_else(|| {
        PipelineState::new(run.run_id.clone(), job.job_id.clone(), base_input.clone())
    }));

    let mut final_state = restored_final_state;
    let mut total_duration_ms: u64 = 0;

    let execution_result: Result<(), OrbitError> = (|| {
        loop {
            if run_was_cancelled(host, &run.run_id)? {
                final_state = JobRunState::Cancelled;
                break;
            }

            let ready: Vec<usize> = {
                let completed_guard = completed.lock().unwrap();
                let degree_guard = remaining_in_degree.lock().unwrap();
                (0..n)
                    .filter(|&index| {
                        let step_id = steps[index]
                            .id
                            .as_deref()
                            .unwrap_or(&steps[index].target_id);
                        degree_guard[index] == 0 && !completed_guard.contains_key(step_id)
                    })
                    .collect()
            };

            if ready.is_empty() {
                let completed_guard = completed.lock().unwrap();
                if completed_guard.len() >= n {
                    break;
                }
                return Err(OrbitError::Execution(
                    "DAG scheduler deadlock: no steps ready but not all completed".to_string(),
                ));
            }

            let results: Result<
                Vec<(
                    usize,
                    String,
                    JobRunState,
                    Option<Value>,
                    Option<i32>,
                    Option<u64>,
                )>,
                OrbitError,
            > = std::thread::scope(|scope| {
                let handles: Vec<_> = ready
                    .iter()
                    .map(|&index| {
                        let step = &steps[index];
                        let step_id = step.id.as_deref().unwrap_or(&step.target_id).to_string();

                        let should_run = {
                            let completed_guard = completed.lock().unwrap();
                            let upstream_states: HashMap<String, JobRunState> = completed_guard
                                .iter()
                                .map(|(id, (state, _))| (id.clone(), *state))
                                .collect();
                            match &step.condition {
                                orbit_types::StepCondition::Expr(_) => {
                                    let current_steps = steps_outputs.lock().unwrap().clone();
                                    let cond_ctx = crate::template::TemplateContext {
                                        input: base_input.clone(),
                                        steps: current_steps,
                                        ..Default::default()
                                    };
                                    super::condition::evaluate_condition(
                                        &step.condition,
                                        &cond_ctx,
                                        |c| {
                                            should_run_dag_step(c, &step.upstream, &upstream_states)
                                        },
                                    )
                                    .unwrap_or(false)
                                }
                                _ => should_run_dag_step(
                                    &step.condition,
                                    &step.upstream,
                                    &upstream_states,
                                ),
                            }
                        };

                        if !should_run {
                            return scope.spawn(move || {
                                Ok((index, step_id, JobRunState::Skipped, None, None, None))
                            });
                        }

                        if run_was_cancelled(host, &run.run_id).unwrap_or(false) {
                            return scope.spawn(move || {
                                Ok((index, step_id, JobRunState::Skipped, None, None, Some(0)))
                            });
                        }

                        let step_input = match build_step_input(step, &base_input) {
                            Ok(value) => value,
                            Err(_) => base_input.clone(),
                        };
                        let current_steps = steps_outputs.lock().unwrap().clone();

                        scope.spawn(move || {
                            let step_started = Utc::now();
                            let resolved_step = resolve_step_agent(host, step, &step_input);
                            let effective_step = resolved_step.as_ref().unwrap_or(step);
                            let execution = match build_execution_context_for_step(
                                host,
                                job,
                                effective_step,
                                step_input,
                                debug,
                                current_steps,
                                Some(&run.run_id),
                                Some(index as u32),
                            ) {
                                Ok(execution) => execution,
                                Err(error) => {
                                    let step_finished = Utc::now();
                                    let _ = host.complete_job_run_step(
                                        &run.run_id,
                                        &JobRunStepParams {
                                            step_index: index,
                                            target_type: step.target_type,
                                            target_id: step.target_id.clone(),
                                            started_at: step_started,
                                            finished_at: step_finished,
                                            duration_ms: Some(0),
                                            exit_code: Some(1),
                                            agent_response_json: None,
                                            state: JobRunState::Failed,
                                            error_code: Some(INPUT_VALIDATION_FAILED.to_string()),
                                            error_message: Some(error.to_string()),
                                        },
                                    );
                                    return Ok((
                                        index,
                                        step_id,
                                        JobRunState::Failed,
                                        None,
                                        Some(1),
                                        Some(0u64),
                                    ));
                                }
                            };

                            if run_was_cancelled(host, &run.run_id)? {
                                return Ok((
                                    index,
                                    step_id,
                                    JobRunState::Skipped,
                                    None,
                                    None,
                                    Some(0),
                                ));
                            }

                            let outcome = execute_with_retry(
                                host,
                                &execution,
                                step.retry_max_attempts,
                                step.retry_backoff_seconds,
                            );
                            let step_finished = Utc::now();
                            let step_state = if outcome.state == JobRunState::Cancelled {
                                JobRunState::Failed
                            } else {
                                outcome.state
                            };

                            let _ = host.complete_job_run_step(
                                &run.run_id,
                                &JobRunStepParams {
                                    step_index: index,
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
                            );

                            let step_output = match execution.state_dir.as_deref() {
                                Some(state_dir) => {
                                    match orbit_store::state_io::read_step_output(
                                        state_dir,
                                        index as u32,
                                    ) {
                                        Ok(output) => output,
                                        Err(error) => return Err(error),
                                    }
                                }
                                None => None,
                            };

                            Ok((
                                index,
                                step_id,
                                step_state,
                                step_output,
                                outcome.exit_code,
                                outcome.duration_ms,
                            ))
                        })
                    })
                    .collect();

                handles
                    .into_iter()
                    .map(|handle| {
                        handle.join().unwrap_or_else(|_| {
                            Err(OrbitError::Execution(
                                "DAG step thread panicked".to_string(),
                            ))
                        })
                    })
                    .collect()
            });

            let results = results?;
            let mut batch_cancelled = false;

            for (index, step_id, state, response, exit_code, duration_ms) in results {
                if state == JobRunState::Cancelled {
                    batch_cancelled = true;
                    final_state = JobRunState::Cancelled;
                } else if state != JobRunState::Success && state != JobRunState::Skipped {
                    final_state = state;
                }

                total_duration_ms += duration_ms.unwrap_or(0);

                let pipeline_patch = if state == JobRunState::Success {
                    response
                        .clone()
                        .map(|output| apply_output_map(output, &steps[index].output_map))
                } else {
                    None
                };

                let entry = wrap_step_entry(Some(state), exit_code, duration_ms, response.as_ref());
                steps_outputs.lock().unwrap().insert(step_id.clone(), entry);

                {
                    let mut state_guard = pipeline_state.lock().unwrap();
                    state_guard.record_step(index as u32, state, response.clone(), pipeline_patch);
                    host.write_run_state(&run.run_id, &state_guard)?;
                }

                completed.lock().unwrap().insert(step_id, (state, response));

                let mut degree_guard = remaining_in_degree.lock().unwrap();
                for &downstream_index in &downstream[index] {
                    degree_guard[downstream_index] =
                        degree_guard[downstream_index].saturating_sub(1);
                }
            }

            if batch_cancelled || run_was_cancelled(host, &run.run_id)? {
                final_state = JobRunState::Cancelled;
                break;
            }
        }
        Ok(())
    })();

    let duration_ms = (total_duration_ms > 0).then_some(total_duration_ms);
    let final_output = pipeline_state.lock().unwrap().pipeline.clone();

    match execution_result {
        Ok(()) => {
            let finished_at = Utc::now();
            let changed =
                host.finalize_job_run(&run.run_id, final_state, finished_at, duration_ms)?;
            if !changed {
                return Err(OrbitError::JobRunNotFound(run.run_id.clone()));
            }
            host.record_event(OrbitEvent::JobRunCompleted {
                job_id: job.job_id.clone(),
                run_id: run.run_id.clone(),
                state: final_state.to_string(),
            })?;
            Ok(JobRunResult {
                job_id: job.job_id.clone(),
                run_id: run.run_id.clone(),
                state: final_state,
                attempt: run.attempt,
                output: Some(final_output),
            })
        }
        Err(error) => {
            if let Some(active_run) = host.get_job_run(&run.run_id)?
                && matches!(
                    active_run.state,
                    JobRunState::Pending | JobRunState::Running
                )
            {
                let finished_at = Utc::now();
                let changed = host.finalize_job_run(
                    &run.run_id,
                    JobRunState::Failed,
                    finished_at,
                    duration_ms,
                )?;
                if changed {
                    host.record_event(OrbitEvent::JobRunCompleted {
                        job_id: job.job_id.clone(),
                        run_id: run.run_id.clone(),
                        state: JobRunState::Failed.to_string(),
                    })?;
                }
            }
            Err(error)
        }
    }
}
