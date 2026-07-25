//! v2 Job DAG executor — the Phase 3 runtime for `JobV2` assets.
//!
//! Interprets a `JobV2` step tree with first-class `parallel:`, `when:`,
//! `retry:`, `fan_out:/fan_in:`, and `loop:` constructs (design §4). The v1
//! sequential/DAG runner in `crate::job_runner` is untouched — this module
//! is purely additive.
//!
//! ## Concurrency
//! Parallel branches and fan-out workers run under `std::thread::scope`
//! (matching v1's DAG scheduler). No tokio, no async.
//!
//! ## Session reuse
//! Loop bodies share a `HashMap<String, Session>`. Target steps with
//! `session: <name>` route through `drive_agent_loop_with_session`, preserving
//! provider conversation history across iterations. Parallel branches /
//! workers that name the same session binding are rejected at validation time
//! — `Session` is `!Sync` by construction and sharing it concurrently would
//! race on `history_mut`.
//!
//! ## Audit
//! Every construct emits §7 envelope events (`step.*`, `fanout.dispatched`,
//! `worker.state`, `fanin.joined`, `loop.iteration.{start,end}`,
//! `loop.did_not_converge`). The retry wrapper emits `step.retry` between
//! attempts and `step.denied` when a denial bypasses retry.

// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use orbit_agent::loop_engine::Session;
use orbit_common::types::activity_job::{
    ActivityV2Spec, AgentLoopSpec, BackoffStrategy, BranchOutcome, FanInSpec, FanOutBlock, JobV2,
    JobV2Step, JobV2StepBody, JoinMode, LoopBlock, ParallelBlock, RetrySpec, TargetStep,
    V2ActivityCatalog, V2AuditEventKind, resolve_job_target_refs,
};
use orbit_common::types::{JobRunState, PipelineState};
use orbit_common::utility::jitter::JitterRng;
use serde_json::Value;

use crate::job_runner::evaluate_bool_expr;
use crate::template::{self, TemplateContext};

use super::agent_loop_driver::drive_agent_loop_with_session;
use super::agent_role::{
    apply_resolved_settings, resolve_agent_settings, resolve_explicit_crew_settings,
};
use super::audit_writer::{V2AuditWriter, WriteError};
use super::dispatcher::{
    DispatchError, V2DispatchInput, V2RuntimeHost, dispatch_v2_activity,
    dispatch_v2_activity_without_run_id_injection,
};

mod audit;
mod concurrency;
mod exec_ctx;
mod fan_out;
mod loop_block;
mod parallel;
mod recovery;
mod step;
mod target;
mod templating;
mod validate;

#[cfg(test)]
mod tests;

use self::audit::*;
use self::concurrency::*;
use self::exec_ctx::*;
use self::fan_out::*;
use self::loop_block::*;
use self::parallel::*;
use self::recovery::*;
use self::step::*;
use self::target::*;
use self::templating::*;

pub use self::validate::validate_job;

#[derive(Debug, Clone)]
pub struct JobOutcome {
    pub success: bool,
    pub pipeline: Value,
    pub message: Option<String>,
    /// [ORB-00414] Number of audit-write failures observed during the run.
    /// Non-zero means the audit trail is incomplete (see `degraded_audit`).
    pub audit_failures: u64,
    /// [ORB-00414] True when any audit write failed — retry/recovery/debugging
    /// consumers should treat the trail as incomplete.
    pub degraded_audit: bool,
    /// [ORB-10367] Number of telemetry-persistence failures (invocation
    /// traces) observed during the run. Never affects `success`.
    pub telemetry_failures: u64,
    /// [ORB-10367] True when any telemetry write failed — the run's
    /// invocation/token accounting is incomplete, but its work is not.
    pub degraded_telemetry: bool,
}

#[derive(Debug, Clone)]
struct ResolvedRecoveryActivity {
    name: String,
    spec: ActivityV2Spec,
}

pub fn resolve_job_catalog_refs_for_execution(
    job: &mut JobV2,
    catalog: &V2ActivityCatalog,
) -> Result<(), DispatchError> {
    resolve_job_target_refs(job, catalog)
        .map_err(|err| DispatchError::JobValidation(err.to_string()))
}

/// Execute a v2 Job against the given host. Mutates pipeline context across
/// steps, writes §7 envelope events through `audit`, and returns the final
/// pipeline map serialized as JSON.
pub fn execute_job(
    job: &JobV2,
    input: Value,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    host: &dyn V2RuntimeHost,
) -> Result<JobOutcome, DispatchError> {
    execute_job_with_resume(job, input, run_id, audit, host, None)
}

/// [ORB-10002] Execute a v2 Job, optionally resuming from a persisted
/// checkpoint state.
///
/// When `resume` is `Some`, top-level steps whose global index is recorded
/// as `success` in `resume.step_states` are skipped (a `step.skipped` audit
/// event is emitted) and their recorded outputs are pre-seeded into the
/// pipeline map so later steps see them through `{{ steps.<id>.output.* }}`
/// templates. Checkpoint granularity is the top-level step: `parallel:` /
/// `fan_out:` / `loop:` blocks re-run as a whole if they did not complete.
/// In-memory agent sessions are not restorable across processes, so resumed
/// steps that share a session start it fresh.
pub fn execute_job_with_resume(
    job: &JobV2,
    input: Value,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    host: &dyn V2RuntimeHost,
    resume: Option<&PipelineState>,
) -> Result<JobOutcome, DispatchError> {
    validate_job(job)?;

    let base_input = merge_job_input(job.default_input.as_ref(), &input);
    let recovery_activity = match (&job.recovery_activity, &job.resolved_recovery_activity) {
        (Some(name), Some(activity)) => Some(ResolvedRecoveryActivity {
            name: name.clone(),
            spec: activity.spec.clone(),
        }),
        _ => None,
    };
    let failure_activity = match (&job.failure_activity, &job.resolved_failure_activity) {
        (Some(name), Some(activity)) => Some(ResolvedRecoveryActivity {
            name: name.clone(),
            spec: activity.spec.clone(),
        }),
        _ => None,
    };

    let ctx = ExecCtx {
        run_id: run_id.to_string(),
        audit: audit.clone(),
        host,
        input: base_input.clone(),
        pipeline: Arc::new(Mutex::new(seed_pipeline_from_resume(job, resume))),
        sessions: Arc::new(Mutex::new(HashMap::new())),
        recovery_activity,
        failure_activity,
        item: None,
        iteration: None,
    };

    let mut overall_ok = true;
    let mut overall_message = None;
    for (index, step) in job.steps.iter().enumerate() {
        let step_index = index as u32;
        if step_completed_in_resume(resume, step_index) {
            emit_job_event_lossy(
                &ctx.audit,
                ctx.task_id(),
                V2AuditEventKind::StepSkipped {
                    step_id: step.id.clone(),
                    reason: format!(
                        "resume: step already completed in checkpointed run (index {step_index})"
                    ),
                },
            );
            continue;
        }
        let outcome = match run_step(step, &ctx) {
            Ok(outcome) => outcome,
            Err(error) => {
                attempt_failure_activity(step, &ctx, &error);
                return Err(error);
            }
        };
        if !outcome.success {
            overall_ok = false;
            overall_message = Some(
                outcome
                    .message
                    .unwrap_or_else(|| format!("step `{}` completed with success=false", step.id)),
            );
            let error = DispatchError::JobExecution(
                overall_message
                    .clone()
                    .unwrap_or_else(|| format!("step `{}` failed", step.id)),
            );
            attempt_failure_activity(step, &ctx, &error);
            break;
        }
        checkpoint_completed_step(&ctx, step_index, &step.id, &outcome.output);
    }

    let pipeline = Value::Object(
        ctx.pipeline
            .lock()
            .expect("pipeline poisoned")
            .clone()
            .into_iter()
            .collect(),
    );

    Ok(JobOutcome {
        success: overall_ok,
        pipeline,
        message: (!overall_ok).then_some(overall_message).flatten(),
        audit_failures: audit.audit_failure_count(),
        degraded_audit: audit.degraded_audit(),
        telemetry_failures: audit.telemetry_failure_count(),
        degraded_telemetry: audit.degraded_telemetry(),
    })
}

/// [ORB-10002] Seed the executor pipeline map from successful checkpoints so
/// skipped steps' outputs stay visible without exposing failed/timed-out data.
fn seed_pipeline_from_resume(
    job: &JobV2,
    resume: Option<&PipelineState>,
) -> HashMap<String, Value> {
    let Some(state) = resume else {
        return HashMap::new();
    };

    job.steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| {
            let step_index = index as u32;
            if state.step_states.get(&step_index) != Some(&JobRunState::Success) {
                return None;
            }
            state
                .step_outputs
                .get(&step_index)
                .cloned()
                .map(|output| (step.id.clone(), output))
        })
        .collect()
}

/// [ORB-10002] True when the resume snapshot records this top-level step as
/// completed successfully; such steps are skipped instead of re-executed.
fn step_completed_in_resume(resume: Option<&PipelineState>, step_index: u32) -> bool {
    resume.is_some_and(|state| state.step_states.get(&step_index) == Some(&JobRunState::Success))
}

/// [ORB-10002] Persist a checkpoint for a completed top-level step through
/// the host. Non-fatal: a checkpoint write failure degrades resumability but
/// must never fail an otherwise-successful run.
fn checkpoint_completed_step(ctx: &ExecCtx<'_>, step_index: u32, step_id: &str, output: &Value) {
    let snapshot = Value::Object(
        ctx.pipeline
            .lock()
            .expect("pipeline poisoned")
            .clone()
            .into_iter()
            .collect(),
    );
    if let Err(error) =
        ctx.host
            .checkpoint_step(&ctx.run_id, step_index, step_id, output, &snapshot)
    {
        tracing::warn!(
            target: "orbit.engine.job_executor",
            run_id = %ctx.run_id,
            step_id,
            step_index,
            error = %error,
            "step checkpoint persistence failed; run continues without a durable checkpoint",
        );
    }
}
