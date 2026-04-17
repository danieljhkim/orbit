use chrono::Utc;
use orbit_types::{Job, JobRun, OrbitError, OrbitEvent};
use serde_json::Value;
use std::path::Path;
use tracing::{info, info_span, warn};

use crate::context::{EngineHost, ExecutorLookupHost, JobRunResult};

use super::pipeline_recovery::build_retry_pipeline_state;
use super::sequential::{ActivityExecutionRequest, execute_activity_with_retries};
use super::stale_recovery::recover_stale_active_run_for_job;

pub fn run_job_with_input<H: EngineHost + ExecutorLookupHost>(
    host: &H,
    data_root: &Path,
    job: Job,
    input: Value,
    debug: bool,
) -> Result<JobRunResult, OrbitError> {
    let job_span = info_span!("job_dispatch", job_id = %job.job_id);
    let _job_span = job_span.enter();
    info!(max_active_runs = job.max_active_runs, "job run requested");

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
            initial_pipeline_state: None,
            debug,
            create_failure_task: true,
            skip_to_step: 0,
            replayed_steps: &[],
            preserve_existing_step_records: false,
        },
    )
}

/// Resume a failed job run from a specific step.
///
/// Creates a NEW run (preserving audit trail). Steps before `retry_step_target_id`
/// are written as Skipped records with replayed outputs from the source run.
/// Execution resumes from the specified step.
pub fn retry_job_run_from_step<H: EngineHost + ExecutorLookupHost>(
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

    let retry_from_index = job
        .steps
        .iter()
        .position(|step| step.target_id == retry_step_target_id)
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "step '{}' not found in job '{}' definition",
                retry_step_target_id, job.job_id
            ))
        })?;

    let source_state = host.read_run_state(&source_run.run_id)?.ok_or_else(|| {
        OrbitError::Store(format!(
            "state.json missing for retry source run '{}'",
            source_run.run_id
        ))
    })?;
    let recovered_state = build_retry_pipeline_state(&job, &source_state, retry_from_index);
    let base_input = recovered_state.pipeline.clone();

    let _ = recover_stale_active_run_for_job(host, data_root, &job, Utc::now())?;
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

    execute_activity_with_retries(
        host,
        data_root,
        job,
        ActivityExecutionRequest {
            scheduled_at: Utc::now(),
            initial_run: None,
            input: base_input,
            initial_pipeline_state: Some(recovered_state),
            debug,
            create_failure_task: true,
            skip_to_step: retry_from_index,
            replayed_steps: &source_run.steps,
            preserve_existing_step_records: false,
        },
    )
}

/// Execute a nested job step by loading the referenced job and running it.
/// Nested jobs do not create their own failure tasks — the outermost pipeline
/// is responsible for creating a single failure task.
pub(super) fn execute_job_step<H: EngineHost + ExecutorLookupHost>(
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
            initial_pipeline_state: None,
            debug,
            create_failure_task: false,
            skip_to_step: 0,
            replayed_steps: &[],
            preserve_existing_step_records: false,
        },
    )
}
