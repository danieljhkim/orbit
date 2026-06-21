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
use serde_json::Value;

use crate::job_runner::evaluate_bool_expr;
use crate::template::{self, TemplateContext};

use super::agent_loop_driver::drive_agent_loop_with_session;
use super::agent_role::{apply_resolved_settings, resolve_agent_settings};
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
    pub error_code: Option<String>,
    pub message: Option<String>,
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
    validate_job(job)?;

    let base_input = merge_job_input(job.default_input.as_ref(), &input);
    let recovery_activity = match (&job.recovery_activity, &job.resolved_recovery_activity) {
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
        pipeline: Arc::new(Mutex::new(HashMap::new())),
        sessions: Arc::new(Mutex::new(HashMap::new())),
        recovery_activity,
        item: None,
        iteration: None,
    };

    let mut overall_ok = true;
    let mut overall_error_code = None;
    let mut overall_message = None;
    for step in &job.steps {
        let outcome = run_step(step, &ctx)?;
        if !outcome.success {
            overall_ok = false;
            overall_error_code = outcome.error_code;
            overall_message = Some(
                outcome
                    .message
                    .unwrap_or_else(|| format!("step `{}` completed with success=false", step.id)),
            );
            break;
        }
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
        error_code: (!overall_ok).then_some(overall_error_code).flatten(),
        message: (!overall_ok).then_some(overall_message).flatten(),
    })
}
