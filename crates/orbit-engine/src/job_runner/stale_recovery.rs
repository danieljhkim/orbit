use chrono::{DateTime, Utc};
use orbit_store::JobRunStepParams;
use orbit_types::{Job, JobRun, JobRunState, OrbitError, OrbitEvent};
use std::path::Path;
#[cfg(unix)]
use std::process::Command;

use crate::context::{
    ACTIVITY_EXECUTION_FAILED, AGENT_INVOCATION_FAILED, JobRunHost, RUN_ABANDONED, RuntimeHost,
    STALE_RUN_GRACE_SECONDS,
};

use super::friction::{FrictionContext, append_failed_step_friction_without_execution};

fn stale_failure_step_index<H: JobRunHost>(host: &H, run: &JobRun) -> usize {
    host.read_run_state(&run.run_id)
        .ok()
        .flatten()
        .map(|state| state.next_step_index as usize)
        .or_else(|| {
            run.steps
                .iter()
                .map(|step| step.step_index as usize)
                .max()
                .map(|index| index.saturating_add(1))
        })
        .unwrap_or(0)
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
            let failure_step_index = stale_failure_step_index(host, &active_run);
            if !job.steps.is_empty()
                && let Some(step) = job.steps.get(failure_step_index % job.steps.len())
            {
                let _ = host.complete_job_run_step(
                    &active_run.run_id,
                    &JobRunStepParams {
                        step_index: failure_step_index,
                        target_type: step.target_type,
                        target_id: step.target_id.clone(),
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
                    &step.target_id,
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

        let failure_step_index = stale_failure_step_index(host, &active_run);
        if !job.steps.is_empty()
            && let Some(step) = job.steps.get(failure_step_index % job.steps.len())
        {
            let _ = host.complete_job_run_step(
                &active_run.run_id,
                &JobRunStepParams {
                    step_index: failure_step_index,
                    target_type: step.target_type,
                    target_id: step.target_id.clone(),
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
                &step.target_id,
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
pub(crate) fn owner_process_missing_or_reused(run: &JobRun) -> bool {
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
pub(crate) fn abandoned_run_message(run: &JobRun, pid: u32) -> String {
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

fn is_stale_active_run(job: &Job, run: &JobRun, now: DateTime<Utc>) -> bool {
    let total_timeout: u64 = job.steps.iter().map(|s| s.timeout_seconds).sum();
    let reference_time = run.started_at.unwrap_or(run.created_at);
    let elapsed_seconds = now.signed_duration_since(reference_time).num_seconds();
    let stale_after_seconds = total_timeout.saturating_add(STALE_RUN_GRACE_SECONDS) as i64;
    elapsed_seconds >= stale_after_seconds
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finalize_failed_started_run<H: JobRunHost + RuntimeHost>(
    host: &H,
    data_root: &Path,
    job: &Job,
    run: &JobRun,
    step_index: usize,
    step: &orbit_types::JobStep,
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
    host.record_event(OrbitEvent::JobRunCompleted {
        job_id: job.job_id.clone(),
        run_id: run.run_id.clone(),
        state: JobRunState::Failed.to_string(),
    })?;
    Ok(())
}
