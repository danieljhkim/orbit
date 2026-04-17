use orbit_types::{Job, JobRunState, OrbitError, OrbitEvent, PipelineState};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::context::{EngineHost, ExecutorLookupHost};
use crate::job_runner::helpers::check_loop_exit;
use crate::job_runner::pipeline_recovery::final_state_from_pipeline_state;
use crate::job_runner::stale_recovery::{abandoned_run_message, owner_process_missing_or_reused};
use crate::job_runner::{ActivityExecutionRequest, execute_activity_with_retries};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconcileOutcome {
    pub runs_processed: usize,
    pub steps_dispatched: usize,
    pub runs_completed: usize,
    pub runs_failed: usize,
    pub errors: Vec<String>,
}

/// Run a single reconciliation pass over all pending/running job runs.
///
/// 1. List pending/running runs across all jobs.
/// 2. Detect and mark abandoned runs (stale PID / PID reuse).
/// 3. Claim ownerless runs and resume them through the normal runner.
pub fn reconcile_once<H: EngineHost + ExecutorLookupHost>(
    host: &H,
    dry_run: bool,
) -> Result<ReconcileOutcome, OrbitError> {
    let mut outcome = ReconcileOutcome {
        runs_processed: 0,
        steps_dispatched: 0,
        runs_completed: 0,
        runs_failed: 0,
        errors: vec![],
    };

    let runs = host.list_all_pending_or_running_runs()?;
    info!(
        run_count = runs.len(),
        "reconcile: loaded pending/running runs"
    );

    for run in &runs {
        outcome.runs_processed += 1;

        // Load the job definition
        let job = match host.get_job(&run.job_id) {
            Ok(Some(job)) => job,
            Ok(None) => {
                outcome
                    .errors
                    .push(format!("job not found: {}", run.job_id));
                continue;
            }
            Err(error) => {
                outcome.errors.push(format!(
                    "run {}: failed to load job '{}': {}",
                    run.run_id, run.job_id, error
                ));
                continue;
            }
        };

        let pipeline_state = match host.read_run_state(&run.run_id) {
            Ok(state) => state,
            Err(error) => {
                outcome.errors.push(format!(
                    "run {}: failed to read persisted state: {}",
                    run.run_id, error
                ));
                continue;
            }
        };
        let persisted_run_is_terminal = pipeline_state
            .as_ref()
            .is_some_and(|state| pipeline_state_is_terminal(host, &job, state));

        if pipeline_state.is_none() && !run.steps.is_empty() {
            let message = format!(
                "run {} has {} recorded step(s) but no persisted state; refusing to replay from step 0",
                run.run_id,
                run.steps.len()
            );
            if dry_run {
                outcome.runs_failed += 1;
                outcome.errors.push(message);
                continue;
            }

            let finished_at = chrono::Utc::now();
            let duration_ms = run.started_at.map(|started| {
                finished_at
                    .signed_duration_since(started)
                    .num_milliseconds()
                    .max(0) as u64
            });
            match host.finalize_job_run(&run.run_id, JobRunState::Failed, finished_at, duration_ms)
            {
                Ok(true) => {
                    if let Err(error) = host.record_event(OrbitEvent::JobRunCompleted {
                        job_id: job.job_id.clone(),
                        run_id: run.run_id.clone(),
                        state: JobRunState::Failed.to_string(),
                    }) {
                        outcome.errors.push(format!(
                            "run {}: finalized after missing state but failed to record event: {}",
                            run.run_id, error
                        ));
                    }
                }
                Ok(false) => {
                    outcome.errors.push(format!(
                        "run {}: missing-state recovery could not finalize the run record",
                        run.run_id
                    ));
                    continue;
                }
                Err(error) => {
                    outcome.errors.push(format!(
                        "run {}: missing-state recovery failed to finalize run: {}",
                        run.run_id, error
                    ));
                    continue;
                }
            }
            outcome.runs_failed += 1;
            outcome.errors.push(message);
            continue;
        }

        // PID-based ownership detection for Running runs.
        if run.state == JobRunState::Running
            && run.pid.is_some()
            && !owner_process_missing_or_reused(run)
        {
            continue;
        }

        if let Some(pid) = run.pid
            && owner_process_missing_or_reused(run)
        {
            if persisted_run_is_terminal {
                info!(
                    run_id = %run.run_id,
                    pid,
                    "reconcile: dead-owner run has terminal persisted state; finalizing through normal runner"
                );
            } else {
                let message = abandoned_run_message(run, pid);
                warn!(
                    run_id = %run.run_id,
                    pid,
                    "reconcile: run abandoned (owner missing or reused)"
                );
                if !dry_run {
                    let finished_at = chrono::Utc::now();
                    let changed = match host.abandon_job_run(&run.run_id, finished_at) {
                        Ok(changed) => changed,
                        Err(error) => {
                            outcome.errors.push(format!(
                                "run {}: failed to abandon run: {}",
                                run.run_id, error
                            ));
                            continue;
                        }
                    };
                    if !changed {
                        outcome.errors.push(format!(
                            "run {}: active run disappeared before it could be abandoned",
                            run.run_id
                        ));
                        continue;
                    }
                    if let Err(error) = host.record_event(OrbitEvent::JobRunCompleted {
                        job_id: job.job_id.clone(),
                        run_id: run.run_id.clone(),
                        state: JobRunState::Failed.to_string(),
                    }) {
                        outcome.errors.push(format!(
                            "run {}: abandoned run but failed to record completion event: {}",
                            run.run_id, error
                        ));
                    }
                }
                outcome.runs_failed += 1;
                outcome
                    .errors
                    .push(format!("run {} abandoned ({message})", run.run_id));
                continue;
            }
        }

        if dry_run {
            outcome.steps_dispatched += 1;
            continue;
        }

        let steps_before = pipeline_state
            .as_ref()
            .map(|state| state.step_states.len())
            .unwrap_or_else(|| run.steps.len());
        let skip_to_step = pipeline_state
            .as_ref()
            .map(|state| state.next_step_index as usize)
            .unwrap_or(0);
        let preserve_existing_step_records = pipeline_state.is_some();
        let run_input = run
            .input
            .clone()
            .unwrap_or_else(|| serde_json::Value::Object(Default::default()));

        info!(
            run_id = %run.run_id,
            job_id = %run.job_id,
            skip_to_step,
            "reconcile: resuming run through normal runner"
        );

        match execute_activity_with_retries(
            host,
            host.data_root(),
            job.clone(),
            ActivityExecutionRequest {
                scheduled_at: run.scheduled_at,
                initial_run: Some(run.clone()),
                input: run_input,
                initial_pipeline_state: pipeline_state,
                debug: false,
                create_failure_task: false,
                skip_to_step,
                replayed_steps: &run.steps,
                preserve_existing_step_records,
            },
        ) {
            Ok(result) => {
                let steps_after = match host.read_run_state(&run.run_id) {
                    Ok(state) => state
                        .map(|state| state.step_states.len())
                        .unwrap_or(steps_before),
                    Err(error) => {
                        outcome.errors.push(format!(
                            "run {}: completed reconcile pass but failed to reload state: {}",
                            run.run_id, error
                        ));
                        steps_before
                    }
                };
                outcome.steps_dispatched += steps_after.saturating_sub(steps_before);
                if result.state == JobRunState::Success {
                    outcome.runs_completed += 1;
                } else {
                    outcome.runs_failed += 1;
                }
            }
            Err(error) => {
                let steps_after = match host.read_run_state(&run.run_id) {
                    Ok(state) => state
                        .map(|state| state.step_states.len())
                        .unwrap_or(steps_before),
                    Err(read_error) => {
                        outcome.errors.push(format!(
                            "run {}: failed to reload state after reconcile error: {}",
                            run.run_id, read_error
                        ));
                        steps_before
                    }
                };
                outcome.steps_dispatched += steps_after.saturating_sub(steps_before);
                match host.get_job_run(&run.run_id) {
                    Ok(Some(current))
                        if current.state.is_terminal() && current.state != JobRunState::Success =>
                    {
                        outcome.runs_failed += 1;
                    }
                    Ok(_) => {}
                    Err(read_error) => {
                        outcome.errors.push(format!(
                            "run {}: reconcile error occurred and run record could not be reloaded: {}",
                            run.run_id, read_error
                        ));
                    }
                }
                outcome
                    .errors
                    .push(format!("run {}: {}", run.run_id, error));
            }
        }
    }

    Ok(outcome)
}

fn pipeline_state_is_terminal<H: EngineHost>(
    host: &H,
    job: &Job,
    pipeline_state: &PipelineState,
) -> bool {
    if job.steps.is_empty() {
        return true;
    }

    if job.steps.iter().any(|step| !step.upstream.is_empty()) {
        return pipeline_state.step_states.len() >= job.steps.len();
    }

    let final_state = final_state_from_pipeline_state(pipeline_state);
    if final_state == JobRunState::Cancelled {
        return true;
    }

    let steps_per_iteration = job.steps.len();
    let next_step_index = pipeline_state.next_step_index as usize;
    if job.max_iterations <= 1 {
        return next_step_index >= steps_per_iteration;
    }

    if check_loop_exit(host, &pipeline_state.pipeline) {
        return true;
    }

    let max_total_steps = steps_per_iteration.saturating_mul(job.max_iterations.max(1) as usize);
    if next_step_index >= max_total_steps {
        return true;
    }

    matches!(final_state, JobRunState::Failed | JobRunState::Timeout)
        && next_step_index > 0
        && next_step_index % steps_per_iteration == 0
}
