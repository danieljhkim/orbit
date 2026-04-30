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
use super::audit_writer::{V2AuditWriter, WriteError};
use super::dispatcher::{
    DispatchError, V2DispatchInput, V2RuntimeHost, dispatch_v2_activity,
    dispatch_v2_activity_without_run_id_injection,
};

const DEFAULT_MODEL_FOR_SESSION: &str = "claude-sonnet-4-5";

/// Result of executing a v2 Job end-to-end.
#[derive(Debug, Clone)]
pub struct JobOutcome {
    pub success: bool,
    pub pipeline: Value,
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
    let mut overall_message = None;
    for step in &job.steps {
        let outcome = run_step(step, &ctx)?;
        if !outcome.success {
            overall_ok = false;
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
        message: (!overall_ok).then_some(overall_message).flatten(),
    })
}

/// Dual-write entry point for job-lifecycle audit events.
///
/// Emits a `tracing::*!` event with a stable, target-keyed projection of
/// `kind` (so the global JSONL feed and `orbit log tail` see live job
/// activity) and persists the same `kind` through the canonical audit writer.
/// Tracing comes first so the live feed reflects the event before the audit
/// store's in-memory snapshot lock is taken; the audit store remains the
/// authoritative trail.
///
/// `task_id` is the optional task identifier extracted from the surrounding
/// activity input (via [`super::cli_runner::task_id_from_input`]) so job-step
/// tracing events correlate with cli_runner subprocess events that already
/// emit `task_id` and `job_run_id`. Pass `None` when no task id is in scope.
///
/// This helper is the only place in `job_executor.rs` that pairs an
/// `audit.emit(...)` with a `tracing::*!` for job-lifecycle events. Adding a
/// new `V2AuditEventKind` variant requires touching only `emit_job_tracing`
/// to get tracing coverage.
fn emit_job_event(
    audit: &V2AuditWriter,
    task_id: Option<&str>,
    kind: V2AuditEventKind,
) -> Result<String, WriteError> {
    emit_job_tracing(audit.run_id(), task_id, &kind);
    audit.emit(kind)
}

/// Project a job-lifecycle `V2AuditEventKind` onto the unified tracing feed
/// using the per-variant target/level rules documented in T20260427-27.
///
/// Common fields on every emission:
/// - `job_run_id` — the `V2AuditWriter::run_id()` (matches cli_runner's field
///   naming so subprocess and step events correlate in JSONL consumers).
/// - `step_id` — present on every job-lifecycle variant.
/// - `task_id` — emitted only when the calling context resolved a task id
///   from the activity input; omitted otherwise.
///
/// Variants that are emitted by other layers (e.g. `RunStarted`,
/// `ActivityStarted`, `FsCall*`, `ToolDenied`, `CliInvocation*`) are
/// intentionally not projected here — those producers run their own dual
/// writes and we do not want job_executor to double-emit on their behalf.
fn emit_job_tracing(job_run_id: &str, task_id: Option<&str>, kind: &V2AuditEventKind) {
    match kind {
        V2AuditEventKind::StepStarted { step_id } => {
            tracing::info!(
                target: "orbit.job.step_started",
                job_run_id = job_run_id,
                task_id = task_id,
                step_id = step_id.as_str(),
                "step started",
            );
        }
        V2AuditEventKind::StepFinished { step_id, outcome } => {
            let success = outcome == "success";
            if success {
                tracing::info!(
                    target: "orbit.job.step_finished",
                    job_run_id = job_run_id,
                    task_id = task_id,
                    step_id = step_id.as_str(),
                    outcome = outcome.as_str(),
                    success = success,
                    "step finished",
                );
            } else {
                tracing::error!(
                    target: "orbit.job.step_finished",
                    job_run_id = job_run_id,
                    task_id = task_id,
                    step_id = step_id.as_str(),
                    outcome = outcome.as_str(),
                    success = success,
                    "step finished",
                );
            }
        }
        V2AuditEventKind::StepSkipped { step_id, reason } => {
            tracing::warn!(
                target: "orbit.job.step_skipped",
                job_run_id = job_run_id,
                task_id = task_id,
                step_id = step_id.as_str(),
                reason = reason.as_str(),
                "step skipped",
            );
        }
        V2AuditEventKind::StepRetry {
            step_id,
            attempt,
            next_backoff_ms,
        } => {
            tracing::warn!(
                target: "orbit.job.step_retry",
                job_run_id = job_run_id,
                task_id = task_id,
                step_id = step_id.as_str(),
                attempt = *attempt,
                next_backoff_ms = *next_backoff_ms,
                "step retry",
            );
        }
        V2AuditEventKind::StepRecoveryAttempted {
            step_id,
            recovery_activity,
            recovery_succeeded,
        } => {
            if *recovery_succeeded {
                tracing::info!(
                    target: "orbit.job.step_recovery_attempted",
                    job_run_id = job_run_id,
                    task_id = task_id,
                    step_id = step_id.as_str(),
                    recovery_activity = recovery_activity.as_str(),
                    recovery_succeeded = *recovery_succeeded,
                    "step recovery attempted",
                );
            } else {
                tracing::warn!(
                    target: "orbit.job.step_recovery_attempted",
                    job_run_id = job_run_id,
                    task_id = task_id,
                    step_id = step_id.as_str(),
                    recovery_activity = recovery_activity.as_str(),
                    recovery_succeeded = *recovery_succeeded,
                    "step recovery attempted",
                );
            }
        }
        V2AuditEventKind::StepDenied { step_id, reason } => {
            tracing::error!(
                target: "orbit.job.step_denied",
                job_run_id = job_run_id,
                task_id = task_id,
                step_id = step_id.as_str(),
                reason = reason.as_str(),
                "step denied",
            );
        }
        V2AuditEventKind::StepJoin {
            step_id,
            mode,
            branch_outcomes,
        } => {
            tracing::info!(
                target: "orbit.job.step_join",
                job_run_id = job_run_id,
                task_id = task_id,
                step_id = step_id.as_str(),
                mode = mode.as_str(),
                branch_count = branch_outcomes.len(),
                "step join",
            );
        }
        V2AuditEventKind::FanoutDispatched {
            step_id,
            worker_count,
        } => {
            tracing::info!(
                target: "orbit.job.fanout",
                job_run_id = job_run_id,
                task_id = task_id,
                step_id = step_id.as_str(),
                phase = "dispatched",
                worker_count = *worker_count,
                "fanout dispatched",
            );
        }
        V2AuditEventKind::FaninJoined {
            step_id,
            collected,
            failed,
        } => {
            tracing::info!(
                target: "orbit.job.fanout",
                job_run_id = job_run_id,
                task_id = task_id,
                step_id = step_id.as_str(),
                phase = "joined",
                collected = *collected,
                failed = *failed,
                "fanin joined",
            );
        }
        V2AuditEventKind::WorkerState {
            step_id,
            worker_index,
            state,
        } => {
            tracing::info!(
                target: "orbit.job.worker_state",
                job_run_id = job_run_id,
                task_id = task_id,
                step_id = step_id.as_str(),
                worker_index = *worker_index,
                state = state.as_str(),
                "worker state",
            );
        }
        V2AuditEventKind::LoopIterationStart { step_id, iteration } => {
            tracing::info!(
                target: "orbit.job.loop_iteration",
                job_run_id = job_run_id,
                task_id = task_id,
                step_id = step_id.as_str(),
                phase = "start",
                iteration = *iteration,
                "loop iteration start",
            );
        }
        V2AuditEventKind::LoopIterationEnd {
            step_id,
            iteration,
            broke,
        } => {
            tracing::info!(
                target: "orbit.job.loop_iteration",
                job_run_id = job_run_id,
                task_id = task_id,
                step_id = step_id.as_str(),
                phase = "end",
                iteration = *iteration,
                broke = *broke,
                "loop iteration end",
            );
        }
        V2AuditEventKind::LoopDidNotConverge {
            step_id,
            max_iterations,
        } => {
            tracing::warn!(
                target: "orbit.job.loop_did_not_converge",
                job_run_id = job_run_id,
                task_id = task_id,
                step_id = step_id.as_str(),
                max_iterations = *max_iterations,
                "loop did not converge",
            );
        }
        // Other V2AuditEventKind variants (RunStarted, RunFinished,
        // ActivityStarted, ActivityFinished, FsCall*, ToolDenied,
        // ToolAllowlistHarnessDelegated, CliInvocationStarted,
        // CliInvocationFinished) are emitted by other producers, which own
        // their own tracing dual-write story. We intentionally do not project
        // them here so call sites outside `job_executor.rs` don't double-emit.
        _ => {}
    }
}

/// Shared execution context for recursive step dispatch.
///
/// `pipeline` accumulates step outputs keyed by step.id. `sessions` holds
/// named `Session` objects live across loop iterations. `item` is populated
/// only inside a fan-out worker so the worker's template context can see
/// `{{ item.* }}`.
struct ExecCtx<'a> {
    run_id: String,
    audit: Arc<V2AuditWriter>,
    host: &'a dyn V2RuntimeHost,
    input: Value,
    pipeline: Arc<Mutex<HashMap<String, Value>>>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    recovery_activity: Option<ResolvedRecoveryActivity>,
    /// `Some(value)` inside a fan-out worker. Rendered into template context
    /// as `{{ item }}`.
    item: Option<Value>,
    iteration: Option<u32>,
}

impl ExecCtx<'_> {
    /// Resolved task id for the activity input, if any. Threaded onto every
    /// job-lifecycle tracing emission so subprocess and step events correlate.
    fn task_id(&self) -> Option<&str> {
        super::cli_runner::task_id_from_input(&self.input)
    }

    fn template_ctx(&self) -> TemplateContext {
        let pipeline = self.pipeline.lock().expect("pipeline poisoned").clone();
        let mut steps: HashMap<String, Value> = HashMap::new();
        for (k, v) in &pipeline {
            steps.insert(k.clone(), wrap_step_output(v));
        }
        let mut input = self.input.clone();
        if let Some(item) = &self.item {
            // Expose item under input.item for template resolution. v1's
            // template engine only splits paths under a named namespace; we
            // reuse the `input.*` namespace to keep the resolver unchanged.
            if let Value::Object(map) = &mut input {
                map.insert("item".to_string(), item.clone());
            }
        }
        if let Some(iteration) = self.iteration
            && let Value::Object(map) = &mut input
        {
            map.insert("iteration".to_string(), Value::from(iteration));
        }
        TemplateContext {
            input,
            env: Default::default(),
            workspace_path: None,
            item: self.item.clone(),
            iteration: self.iteration,
            steps,
        }
    }
}

/// Pipeline outputs are stored raw but the template engine expects
/// `{{ steps.<id>.output.<field> }}`. Wrap them accordingly so callers read
/// with the same `.output.` prefix they would against a v1 step.
fn wrap_step_output(raw: &Value) -> Value {
    serde_json::json!({ "output": raw })
}

/// Result of running a single step.
struct StepOutcome {
    success: bool,
    output: Value,
    message: Option<String>,
    /// `true` when `when:` returned false and the step did not run. Kept for
    /// future callers that need to distinguish a skipped-but-successful step
    /// from one that actually executed.
    #[allow(dead_code)]
    skipped: bool,
}

fn run_step(step: &JobV2Step, ctx: &ExecCtx<'_>) -> Result<StepOutcome, DispatchError> {
    // `when:` evaluated once, before retry. A skipped step doesn't retry.
    if let Some(expr) = &step.when {
        let tctx = ctx.template_ctx();
        let matched = evaluate_bool_expr(expr, &tctx)
            .map_err(|err| DispatchError::JobExecution(format!("when expr: {err}")))?;
        if !matched {
            let _ = emit_job_event(
                &ctx.audit,
                ctx.task_id(),
                V2AuditEventKind::StepSkipped {
                    step_id: step.id.clone(),
                    reason: format!("when:{expr} => false"),
                },
            );
            return Ok(StepOutcome {
                success: true,
                output: Value::Null,
                message: None,
                skipped: true,
            });
        }
    }

    let step_event_id = emit_job_event(
        &ctx.audit,
        ctx.task_id(),
        V2AuditEventKind::StepStarted {
            step_id: step.id.clone(),
        },
    )
    .map_err(|e| DispatchError::AuditFailed(format!("{e:?}")))?;
    let _ = ctx.audit.push_parent(step_event_id);

    let result = run_step_with_retry(step, ctx);

    let _ = ctx.audit.pop_parent();
    let outcome_str = match &result {
        Ok(StepOutcome { success: true, .. }) => "success",
        Ok(_) => "failed",
        Err(_) => "error",
    };
    let _ = emit_job_event(
        &ctx.audit,
        ctx.task_id(),
        V2AuditEventKind::StepFinished {
            step_id: step.id.clone(),
            outcome: outcome_str.to_string(),
        },
    );

    result
}

fn run_step_with_retry(step: &JobV2Step, ctx: &ExecCtx<'_>) -> Result<StepOutcome, DispatchError> {
    let Some(retry) = &step.retry else {
        // No retry wrapper — single attempt.
        return match run_step_body(step, ctx) {
            Ok(outcome) => Ok(outcome),
            Err(err) if err.is_non_retryable() => {
                emit_denied_if_applicable(&err, &step.id, &ctx.audit, ctx.task_id());
                Err(err)
            }
            Err(err) => recover_or_return_original(step, ctx, err, 1, 1),
        };
    };

    let mut last_err: Option<DispatchError> = None;
    let max_attempts = retry.max_attempts.max(1);
    for attempt in 0..max_attempts {
        match run_step_body(step, ctx) {
            Ok(outcome) => {
                if outcome.success {
                    return Ok(outcome);
                }
                // Treat a "not-success-but-no-error" outcome (e.g. shell
                // exited non-zero) as retryable: another attempt may succeed.
                last_err = None;
            }
            Err(err) if err.is_non_retryable() => {
                emit_denied_if_applicable(&err, &step.id, &ctx.audit, ctx.task_id());
                return Err(err);
            }
            Err(err) => {
                last_err = Some(err);
            }
        }
        if attempt + 1 >= max_attempts {
            break;
        }
        let backoff_ms = compute_backoff_ms(retry, attempt);
        let _ = emit_job_event(
            &ctx.audit,
            ctx.task_id(),
            V2AuditEventKind::StepRetry {
                step_id: step.id.clone(),
                attempt: attempt + 1,
                next_backoff_ms: backoff_ms,
            },
        );
        thread::sleep(Duration::from_millis(backoff_ms));
    }

    match last_err {
        Some(err) => recover_or_return_original(step, ctx, err, max_attempts, max_attempts),
        None => Ok(StepOutcome {
            success: false,
            output: Value::Null,
            message: None,
            skipped: false,
        }),
    }
}

fn recover_or_return_original(
    step: &JobV2Step,
    ctx: &ExecCtx<'_>,
    original_err: DispatchError,
    attempt: u32,
    max_attempts: u32,
) -> Result<StepOutcome, DispatchError> {
    let Some(recovery) = &ctx.recovery_activity else {
        return Err(original_err);
    };

    if attempt_recovery_activity(step, ctx, recovery, &original_err, attempt, max_attempts) {
        match run_step_body(step, ctx) {
            Ok(outcome) if outcome.success => return Ok(outcome),
            Ok(_) | Err(_) => {}
        }
    }

    Err(original_err)
}

fn attempt_recovery_activity(
    step: &JobV2Step,
    ctx: &ExecCtx<'_>,
    recovery: &ResolvedRecoveryActivity,
    original_err: &DispatchError,
    attempt: u32,
    max_attempts: u32,
) -> bool {
    let input = serde_json::json!({
        "failed_step_id": step.id,
        "activity_name": step_activity_name(step),
        "error_message": original_err.to_string(),
        "attempt": attempt,
        "max_attempts": max_attempts,
    });
    let dispatch = dispatch_v2_activity_without_run_id_injection(V2DispatchInput {
        activity_name: &recovery.name,
        spec: &recovery.spec,
        fs_profile: step_fs_profile(step),
        input: input.clone(),
        audit: ctx.audit.clone(),
        run_id: &ctx.run_id,
        host: Some(ctx.host),
    });

    let recovery_succeeded = match dispatch {
        Ok(dispatch) if dispatch.success => {
            let _ = persist_dispatch_invocation(ctx, &recovery.name, &input, &dispatch);
            true
        }
        Ok(dispatch) => {
            let _ = persist_dispatch_invocation(ctx, &recovery.name, &input, &dispatch);
            false
        }
        Err(_) => false,
    };

    let _ = emit_job_event(
        &ctx.audit,
        ctx.task_id(),
        V2AuditEventKind::StepRecoveryAttempted {
            step_id: step.id.clone(),
            recovery_activity: recovery.name.clone(),
            recovery_succeeded,
        },
    );

    recovery_succeeded
}

fn step_fs_profile(step: &JobV2Step) -> Option<&str> {
    match &step.body {
        JobV2StepBody::Target(target) => target.fs_profile.as_deref(),
        _ => None,
    }
}

fn step_activity_name(step: &JobV2Step) -> String {
    match &step.body {
        JobV2StepBody::Target(target) => target
            .activity_name
            .clone()
            .unwrap_or_else(|| target_activity_label(target)),
        JobV2StepBody::TargetRef(target) => target.target.clone(),
        JobV2StepBody::Parallel { .. } => "parallel".to_string(),
        JobV2StepBody::FanOut { .. } => "fan_out".to_string(),
        JobV2StepBody::Loop { .. } => "loop".to_string(),
    }
}

fn target_activity_label(target: &TargetStep) -> String {
    match &target.spec {
        ActivityV2Spec::AgentLoop(_) => "agent_loop".to_string(),
        ActivityV2Spec::Groundhog(_) => "groundhog".to_string(),
        ActivityV2Spec::Deterministic(spec) => spec.action.clone(),
        ActivityV2Spec::Shell(spec) => format!("shell:{}", spec.program),
    }
}

fn compute_backoff_ms(retry: &RetrySpec, attempt_index: u32) -> u64 {
    match retry.backoff_strategy {
        BackoffStrategy::Exponential => {
            let shifted = retry
                .initial_backoff_ms
                .saturating_mul(1u64 << attempt_index.min(20));
            shifted.min(retry.backoff_cap_ms)
        }
        BackoffStrategy::Linear => retry
            .initial_backoff_ms
            .saturating_mul(attempt_index as u64 + 1)
            .min(retry.backoff_cap_ms),
    }
}

fn emit_denied_if_applicable(
    err: &DispatchError,
    step_id: &str,
    audit: &Arc<V2AuditWriter>,
    task_id: Option<&str>,
) {
    if matches!(err, DispatchError::ToolDenied { .. }) {
        let _ = emit_job_event(
            audit,
            task_id,
            V2AuditEventKind::StepDenied {
                step_id: step_id.to_string(),
                reason: err.to_string(),
            },
        );
    }
}

fn run_step_body(step: &JobV2Step, ctx: &ExecCtx<'_>) -> Result<StepOutcome, DispatchError> {
    match &step.body {
        JobV2StepBody::Target(t) => run_target(step, t, ctx),
        JobV2StepBody::TargetRef(r) => Err(DispatchError::JobValidation(format!(
            "step `{}`: target ref `{}` was not resolved — caller must run \
             `resolve_job_target_refs` at load time before dispatch",
            step.id, r.target
        ))),
        JobV2StepBody::Parallel { parallel } => run_parallel(step, parallel, ctx),
        JobV2StepBody::FanOut { fan_out, fan_in } => run_fan_out(step, fan_out, fan_in, ctx),
        JobV2StepBody::Loop { loop_ } => run_loop(step, loop_, ctx),
    }
}

// ---------------------------------------------------------------------------
// Target
// ---------------------------------------------------------------------------

fn run_target(
    step: &JobV2Step,
    t: &TargetStep,
    ctx: &ExecCtx<'_>,
) -> Result<StepOutcome, DispatchError> {
    let tctx = ctx.template_ctx();
    let rendered_input = render_input(t.default_input.as_ref(), &ctx.input, &tctx)?;

    match (&t.spec, &t.session) {
        (ActivityV2Spec::AgentLoop(agent_spec), Some(binding)) => {
            // Sessions only bind in HTTP mode; the loader rejects `loop +
            // session + cli` via `validate_job_loop_session_backends`, but we
            // also guard structurally here in case a flat (non-loop) target
            // declares session + cli.
            if agent_spec.backend != orbit_common::types::activity_job::Backend::Http {
                return Err(DispatchError::JobValidation(format!(
                    "step `{}`: `session:` binding requires backend: http (got {})",
                    step.id,
                    agent_spec.backend.as_str()
                )));
            }
            if !agent_spec.provider.has_http_transport() {
                return Err(DispatchError::UnwiredHttpTransport {
                    provider: agent_spec.provider.as_str().to_string(),
                });
            }
            // Reuse the named Session across calls. Held under a Mutex so
            // siblings could theoretically reference the same binding; the
            // validator rejects that shape, but the lock is cheap.
            let mut sessions = ctx.sessions.lock().expect("sessions poisoned");
            let entry = sessions.entry(binding.clone()).or_insert_with(|| {
                let model = agent_spec
                    .model
                    .clone()
                    .unwrap_or_else(|| DEFAULT_MODEL_FOR_SESSION.to_string());
                let provider = if replay_active() {
                    "replay"
                } else {
                    "anthropic"
                };
                Session::new(provider, model, &agent_spec.instruction, None)
            });
            let api_key = ctx.host.api_key_for("anthropic").ok();
            run_agent_loop_outcome(
                step,
                agent_spec,
                api_key.as_deref(),
                entry,
                &rendered_input,
                t.fs_profile.as_deref(),
                ctx,
            )
        }
        (ActivityV2Spec::AgentLoop(_agent_spec), None) => {
            // Route through the backend-aware dispatcher so a step with
            // `backend: cli` lands on the CLI runner rather than the HTTP
            // driver.
            let dispatch = dispatch_v2_activity(V2DispatchInput {
                activity_name: &step.id,
                spec: &t.spec,
                fs_profile: t.fs_profile.as_deref(),
                input: rendered_input.clone(),
                audit: ctx.audit.clone(),
                run_id: &ctx.run_id,
                host: Some(ctx.host),
            })?;
            persist_dispatch_invocation(ctx, &step.id, &rendered_input, &dispatch)?;
            let out = dispatch.output.clone();
            record_pipeline(ctx, &step.id, out.clone());
            Ok(StepOutcome {
                success: dispatch.success,
                output: out,
                message: dispatch.message,
                skipped: false,
            })
        }
        _ => {
            // Deterministic / Shell dispatch via the existing Phase 2 entry.
            let dispatch = dispatch_v2_activity(V2DispatchInput {
                activity_name: &step.id,
                spec: &t.spec,
                fs_profile: t.fs_profile.as_deref(),
                input: rendered_input.clone(),
                audit: ctx.audit.clone(),
                run_id: &ctx.run_id,
                host: Some(ctx.host),
            })?;
            persist_dispatch_invocation(ctx, &step.id, &rendered_input, &dispatch)?;
            let out = dispatch.output.clone();
            record_pipeline(ctx, &step.id, out.clone());
            Ok(StepOutcome {
                success: dispatch.success,
                output: out,
                message: dispatch.message,
                skipped: false,
            })
        }
    }
}

fn persist_dispatch_invocation(
    ctx: &ExecCtx<'_>,
    step_id: &str,
    input: &Value,
    dispatch: &super::dispatcher::DispatchOutcome,
) -> Result<(), DispatchError> {
    let Some(invocation) = dispatch.invocation.as_ref() else {
        return Ok(());
    };

    ctx.host.persist_invocation_trace(
        &ctx.run_id,
        step_id,
        &invocation.provider,
        invocation.model.as_deref(),
        input,
        &invocation.trace,
    )
}

fn run_agent_loop_outcome(
    step: &JobV2Step,
    spec: &AgentLoopSpec,
    api_key: Option<&str>,
    session: &mut Session,
    input: &Value,
    fs_profile: Option<&str>,
    ctx: &ExecCtx<'_>,
) -> Result<StepOutcome, DispatchError> {
    let started = Instant::now();
    let outcome = drive_agent_loop_with_session(
        spec,
        api_key,
        &ctx.run_id,
        ctx.audit.clone(),
        session,
        input,
        ctx.host,
        fs_profile,
    )?;
    let trace =
        super::dispatcher::loop_outcome_trace(&outcome, started.elapsed().as_millis() as u64);
    ctx.host.persist_invocation_trace(
        &ctx.run_id,
        &step.id,
        spec.provider.as_str(),
        spec.model.as_deref(),
        input,
        &trace,
    )?;
    let out_json = serde_json::json!({
        "final_message": outcome.final_message,
        "terminate_reason": format!("{:?}", outcome.terminate_reason),
    });
    record_pipeline(ctx, &step.id, out_json.clone());
    Ok(StepOutcome {
        success: true,
        output: out_json,
        message: None,
        skipped: false,
    })
}

fn replay_active() -> bool {
    std::env::var("ORBIT_V2_REPLAY").is_ok() || std::env::var("ORBIT_V2_REPLAY_FIXTURE").is_ok()
}

fn render_input(
    default_input: Option<&Value>,
    base_input: &Value,
    tctx: &TemplateContext,
) -> Result<Value, DispatchError> {
    let src = default_input.cloned().unwrap_or_else(|| base_input.clone());
    render_value(&src, tctx)
}

fn merge_job_input(default_input: Option<&Value>, input: &Value) -> Value {
    match (default_input, input) {
        (Some(defaults), Value::Null) => defaults.clone(),
        (Some(Value::Object(defaults)), Value::Object(explicit)) => {
            let mut merged = defaults.clone();
            for (key, value) in explicit {
                merged.insert(key.clone(), value.clone());
            }
            Value::Object(merged)
        }
        _ => input.clone(),
    }
}

fn render_items_expression(
    expression: &str,
    tctx: &TemplateContext,
    label: &str,
) -> Result<Vec<Value>, DispatchError> {
    let rendered = template::render(expression, tctx)
        .map_err(|err| DispatchError::JobExecution(format!("{label} render: {err}")))?;
    Ok(serde_json::from_str(&rendered).unwrap_or_else(|_| {
        rendered
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|segment| !segment.is_empty())
            .map(|segment| Value::String(segment.to_string()))
            .collect()
    }))
}

/// Recursive template render: resolves `{{ ... }}` tokens in any string
/// within a JSON tree. Non-strings pass through unchanged.
fn render_value(v: &Value, tctx: &TemplateContext) -> Result<Value, DispatchError> {
    match v {
        Value::String(s) if s.contains("{{") => {
            let rendered = template::render(s, tctx)
                .map_err(|err| DispatchError::JobExecution(format!("template render: {err}")))?;
            // Try to parse back to a JSON value (numbers, bools, arrays);
            // fall back to string if parse fails.
            Ok(serde_json::from_str::<Value>(&rendered).unwrap_or(Value::String(rendered)))
        }
        Value::Array(arr) => {
            let out: Result<Vec<_>, _> = arr.iter().map(|x| render_value(x, tctx)).collect();
            Ok(Value::Array(out?))
        }
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), render_value(v, tctx)?);
            }
            Ok(Value::Object(out))
        }
        _ => Ok(v.clone()),
    }
}

fn record_pipeline(ctx: &ExecCtx<'_>, key: &str, v: Value) {
    ctx.pipeline
        .lock()
        .expect("pipeline poisoned")
        .insert(key.to_string(), v);
}

// ---------------------------------------------------------------------------
// Parallel
// ---------------------------------------------------------------------------

fn run_parallel(
    step: &JobV2Step,
    block: &ParallelBlock,
    ctx: &ExecCtx<'_>,
) -> Result<StepOutcome, DispatchError> {
    let branches = &block.branches;
    if branches.is_empty() {
        return Ok(StepOutcome {
            success: true,
            output: Value::Array(Vec::new()),
            message: None,
            skipped: false,
        });
    }
    let inherited_parent_stack = ctx
        .audit
        .parent_stack_snapshot()
        .map_err(|err| DispatchError::AuditFailed(format!("{err:?}")))?;

    let results: Vec<(String, Result<StepOutcome, DispatchError>)> = thread::scope(|scope| {
        let handles: Vec<_> = branches
            .iter()
            .map(|branch| {
                let branch_id = branch.id.clone();
                let ctx_ref = ctx;
                let audit = ctx.audit.clone();
                let inherited_parent_stack = inherited_parent_stack.clone();
                scope.spawn(move || {
                    let _parent_guard = match audit.install_parent_stack(inherited_parent_stack) {
                        Ok(guard) => guard,
                        Err(err) => {
                            return (
                                branch_id,
                                Err(DispatchError::AuditFailed(format!("{err:?}"))),
                            );
                        }
                    };
                    (branch_id, run_step(branch, ctx_ref))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("branch thread panicked"))
            .collect()
    });

    let mut outcomes: Vec<BranchOutcome> = Vec::new();
    let mut successes = 0u32;
    let mut failures = 0u32;
    let mut first_error: Option<DispatchError> = None;
    for (branch_id, res) in &results {
        let outcome_str = match res {
            Ok(o) if o.success => "success",
            Ok(_) => "failed",
            Err(_) => "error",
        };
        match res {
            Ok(o) if o.success => successes += 1,
            Ok(_) => failures += 1,
            Err(e) => {
                failures += 1;
                if first_error.is_none() {
                    first_error = Some(e.clone());
                }
            }
        }
        outcomes.push(BranchOutcome {
            branch_id: branch_id.clone(),
            outcome: outcome_str.to_string(),
        });
    }

    let mode_label = match &block.join {
        JoinMode::All => "all",
        JoinMode::Any => "any",
        JoinMode::Quorum { .. } => "quorum",
    };
    let _ = emit_job_event(
        &ctx.audit,
        ctx.task_id(),
        V2AuditEventKind::StepJoin {
            step_id: step.id.clone(),
            mode: mode_label.to_string(),
            branch_outcomes: outcomes.clone(),
        },
    );

    let block_ok = match &block.join {
        JoinMode::All => failures == 0,
        JoinMode::Any => successes > 0,
        JoinMode::Quorum { n } => successes >= *n,
    };

    if !block_ok && let Some(err) = first_error {
        // Surface the first branch error if no branch succeeded and any
        // errored — gives the caller a structural reason, not a bare false.
        return Err(err);
    }

    let branch_json: Vec<Value> = outcomes
        .into_iter()
        .map(|b| serde_json::json!({"branch_id": b.branch_id, "outcome": b.outcome}))
        .collect();
    let out = Value::Array(branch_json);
    record_pipeline(ctx, &step.id, out.clone());

    Ok(StepOutcome {
        success: block_ok,
        output: out,
        message: None,
        skipped: false,
    })
}

// ---------------------------------------------------------------------------
// Fan-out / fan-in
// ---------------------------------------------------------------------------

fn run_fan_out(
    step: &JobV2Step,
    block: &FanOutBlock,
    fan_in: &FanInSpec,
    ctx: &ExecCtx<'_>,
) -> Result<StepOutcome, DispatchError> {
    let tctx = ctx.template_ctx();
    let items = render_items_expression(&block.items, &tctx, "fan_out.items")?;
    let worker_count = items.len() as u32;

    let _ = emit_job_event(
        &ctx.audit,
        ctx.task_id(),
        V2AuditEventKind::FanoutDispatched {
            step_id: step.id.clone(),
            worker_count,
        },
    );

    if items.is_empty() {
        let _ = emit_job_event(
            &ctx.audit,
            ctx.task_id(),
            V2AuditEventKind::FaninJoined {
                step_id: step.id.clone(),
                collected: 0,
                failed: 0,
            },
        );
        return Ok(StepOutcome {
            success: true,
            output: Value::Array(Vec::new()),
            message: None,
            skipped: false,
        });
    }

    let inherited_parent_stack = ctx
        .audit
        .parent_stack_snapshot()
        .map_err(|err| DispatchError::AuditFailed(format!("{err:?}")))?;
    let semaphore = Arc::new(Semaphore::new(block.max_workers.max(1) as usize));
    let results: Mutex<Vec<(u32, Result<StepOutcome, DispatchError>)>> =
        Mutex::new(Vec::with_capacity(items.len()));

    thread::scope(|scope| {
        let mut handles = Vec::new();
        for (idx, item) in items.iter().enumerate() {
            let idx = idx as u32;
            let sem = Arc::clone(&semaphore);
            let item = item.clone();
            let worker_step = block.worker.clone();
            let run_id = ctx.run_id.clone();
            let audit = ctx.audit.clone();
            let host = ctx.host;
            let base_input = ctx.input.clone();
            let pipeline_snapshot = ctx.pipeline.lock().expect("pipeline poisoned").clone();
            let results_ref = &results;
            let inherited_parent_stack = inherited_parent_stack.clone();

            handles.push(scope.spawn(move || {
                let _parent_guard = match audit.install_parent_stack(inherited_parent_stack) {
                    Ok(guard) => guard,
                    Err(err) => {
                        results_ref
                            .lock()
                            .expect("results poisoned")
                            .push((idx, Err(DispatchError::AuditFailed(format!("{err:?}")))));
                        return;
                    }
                };
                let _permit = sem.acquire();
                // Resolve task_id once so both worker_state emissions stamp
                // the same value; `base_input` will be moved into worker_ctx
                // below and is no longer accessible afterwards.
                let worker_task_id =
                    super::cli_runner::task_id_from_input(&base_input).map(str::to_string);
                let _ = emit_job_event(
                    &audit,
                    worker_task_id.as_deref(),
                    V2AuditEventKind::WorkerState {
                        step_id: worker_step.id.clone(),
                        worker_index: idx,
                        state: "dispatched".to_string(),
                    },
                );
                let worker_ctx = ExecCtx {
                    run_id,
                    audit: audit.clone(),
                    host,
                    input: base_input,
                    pipeline: Arc::new(Mutex::new(pipeline_snapshot)),
                    sessions: Arc::new(Mutex::new(HashMap::new())),
                    recovery_activity: ctx.recovery_activity.clone(),
                    item: Some(item),
                    iteration: Some(idx),
                };
                let res = run_step(&worker_step, &worker_ctx);
                let state = match &res {
                    Ok(o) if o.success => "finished",
                    Ok(_) => "failed",
                    Err(_) => "failed",
                };
                let _ = emit_job_event(
                    &audit,
                    worker_task_id.as_deref(),
                    V2AuditEventKind::WorkerState {
                        step_id: worker_step.id.clone(),
                        worker_index: idx,
                        state: state.to_string(),
                    },
                );
                results_ref
                    .lock()
                    .expect("results poisoned")
                    .push((idx, res));
            }));
        }
        for h in handles {
            let _ = h.join();
        }
    });

    let mut collected: Vec<Value> = Vec::new();
    let mut collected_count = 0u32;
    let mut failed_count = 0u32;
    let mut first_error: Option<DispatchError> = None;
    let mut sorted = results.into_inner().expect("results poisoned");
    sorted.sort_by_key(|(idx, _)| *idx);
    for (_idx, res) in sorted {
        match res {
            Ok(o) if o.success => {
                collected_count += 1;
                collected.push(o.output);
            }
            Ok(_) => {
                failed_count += 1;
                collected.push(Value::Null);
            }
            Err(e) => {
                failed_count += 1;
                if first_error.is_none() {
                    first_error = Some(e);
                }
                collected.push(Value::Null);
            }
        }
    }

    let _ = emit_job_event(
        &ctx.audit,
        ctx.task_id(),
        V2AuditEventKind::FaninJoined {
            step_id: step.id.clone(),
            collected: collected_count,
            failed: failed_count,
        },
    );

    let block_ok = match &fan_in.join {
        JoinMode::All => failed_count == 0,
        JoinMode::Any => collected_count > 0,
        JoinMode::Quorum { n } => collected_count >= *n,
    };

    if !block_ok && let Some(err) = first_error {
        return Err(err);
    }

    let collected_value = Value::Array(collected);
    if let Some(collect_key) = &fan_in.collect {
        record_pipeline(ctx, collect_key, collected_value.clone());
    }
    record_pipeline(ctx, &step.id, collected_value.clone());

    Ok(StepOutcome {
        success: block_ok,
        output: collected_value,
        message: None,
        skipped: false,
    })
}

// ---------------------------------------------------------------------------
// Loop
// ---------------------------------------------------------------------------

fn run_loop(
    step: &JobV2Step,
    block: &LoopBlock,
    ctx: &ExecCtx<'_>,
) -> Result<StepOutcome, DispatchError> {
    let loop_items = match &block.items {
        Some(expression) => Some(render_items_expression(
            expression,
            &ctx.template_ctx(),
            "loop.items",
        )?),
        None => None,
    };
    if let Some(items) = &loop_items
        && items.len() > block.max_iterations as usize
    {
        return Err(DispatchError::JobExecution(format!(
            "loop.items produced {} entries, exceeding max_iterations {}",
            items.len(),
            block.max_iterations
        )));
    }

    let mut broke = false;
    let mut last_iter: u32 = 0;
    let planned_iterations = loop_items
        .as_ref()
        .map(|items| items.len() as u32)
        .unwrap_or(block.max_iterations);
    for iter in 1..=planned_iterations {
        let iteration_index = iter - 1;
        last_iter = iter;
        let _ = emit_job_event(
            &ctx.audit,
            ctx.task_id(),
            V2AuditEventKind::LoopIterationStart {
                step_id: step.id.clone(),
                iteration: iter,
            },
        );
        let loop_ctx = ExecCtx {
            run_id: ctx.run_id.clone(),
            audit: ctx.audit.clone(),
            host: ctx.host,
            input: ctx.input.clone(),
            pipeline: ctx.pipeline.clone(),
            sessions: ctx.sessions.clone(),
            recovery_activity: ctx.recovery_activity.clone(),
            item: loop_items
                .as_ref()
                .and_then(|items| items.get(iteration_index as usize).cloned()),
            iteration: Some(iteration_index),
        };

        for body in &block.steps {
            let outcome = run_step(body, &loop_ctx)?;
            if !outcome.success {
                return Ok(StepOutcome {
                    success: false,
                    output: outcome.output,
                    message: outcome.message,
                    skipped: false,
                });
            }
        }

        // Evaluate break_when after the body runs so the body can populate
        // pipeline state that the expression references.
        let should_break = if let Some(expr) = &block.break_when {
            let tctx = loop_ctx.template_ctx();
            evaluate_bool_expr(expr, &tctx)
                .map_err(|err| DispatchError::JobExecution(format!("break_when: {err}")))?
        } else {
            false
        };

        let _ = emit_job_event(
            &ctx.audit,
            ctx.task_id(),
            V2AuditEventKind::LoopIterationEnd {
                step_id: step.id.clone(),
                iteration: iter,
                broke: should_break,
            },
        );

        if should_break {
            broke = true;
            break;
        }
    }

    if !broke && block.break_when.is_some() {
        let _ = emit_job_event(
            &ctx.audit,
            ctx.task_id(),
            V2AuditEventKind::LoopDidNotConverge {
                step_id: step.id.clone(),
                max_iterations: block.max_iterations,
            },
        );
    }

    let out = serde_json::json!({
        "iterations": last_iter,
        "broke": broke,
    });
    record_pipeline(ctx, &step.id, out.clone());

    Ok(StepOutcome {
        success: true,
        output: out,
        message: None,
        skipped: false,
    })
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Pre-execution structural checks. Phase 3's one hard rule:
/// concurrent siblings must not name the same `session:` binding — `Session`
/// is `!Sync` and sharing it would race on history_mut.
pub fn validate_job(job: &JobV2) -> Result<(), DispatchError> {
    if let Some(name) = &job.recovery_activity
        && job.resolved_recovery_activity.is_none()
    {
        return Err(DispatchError::JobValidation(format!(
            "job recovery_activity `{name}` was not resolved — caller must run \
             `resolve_job_catalog_refs_for_execution` at load time before dispatch"
        )));
    }
    for step in &job.steps {
        validate_step(step)?;
    }
    Ok(())
}

fn validate_step(step: &JobV2Step) -> Result<(), DispatchError> {
    match &step.body {
        JobV2StepBody::Parallel { parallel } => {
            let mut seen: HashMap<&str, &str> = HashMap::new();
            for branch in &parallel.branches {
                collect_session_bindings(branch, &mut seen, &step.id)?;
                validate_step(branch)?;
            }
        }
        JobV2StepBody::FanOut { fan_out, .. } => {
            // Workers execute concurrently against one another. If the worker
            // template names a session binding, every worker would share it.
            let mut seen: HashMap<&str, &str> = HashMap::new();
            collect_session_bindings(&fan_out.worker, &mut seen, &step.id)?;
            if !seen.is_empty() {
                return Err(DispatchError::JobValidation(format!(
                    "fan_out step `{}` worker names session binding(s); workers run concurrently and Session is !Sync",
                    step.id
                )));
            }
            validate_step(&fan_out.worker)?;
        }
        JobV2StepBody::Loop { loop_ } => {
            for body in &loop_.steps {
                validate_step(body)?;
            }
        }
        JobV2StepBody::Target(_) => {}
        JobV2StepBody::TargetRef(_) => {
            // Surfaces as a structural error in `run_step_body`; no session
            // binding to validate here.
        }
    }
    Ok(())
}

fn collect_session_bindings<'a>(
    step: &'a JobV2Step,
    seen: &mut HashMap<&'a str, &'a str>,
    parent_id: &'a str,
) -> Result<(), DispatchError> {
    match &step.body {
        JobV2StepBody::Target(t) => {
            if let Some(binding) = &t.session {
                let name: &str = binding.as_str();
                if let Some(other) = seen.insert(name, step.id.as_str()) {
                    return Err(DispatchError::JobValidation(format!(
                        "parallel siblings under `{parent_id}` both bind session `{name}`: steps `{other}` and `{}`",
                        step.id
                    )));
                }
            }
        }
        JobV2StepBody::Parallel { parallel } => {
            for b in &parallel.branches {
                collect_session_bindings(b, seen, parent_id)?;
            }
        }
        JobV2StepBody::FanOut { fan_out, .. } => {
            collect_session_bindings(&fan_out.worker, seen, parent_id)?;
        }
        JobV2StepBody::Loop { loop_ } => {
            for b in &loop_.steps {
                collect_session_bindings(b, seen, parent_id)?;
            }
        }
        JobV2StepBody::TargetRef(_) => {
            // Can't collect bindings from an unresolved ref; dispatcher
            // surfaces the structural error.
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Semaphore
// ---------------------------------------------------------------------------

/// Minimal counting semaphore used by fan_out to bound concurrent workers.
/// Uses Mutex+Condvar so it can be acquired/released from std::thread::scope
/// worker closures without pulling a new dependency in.
struct Semaphore {
    state: Mutex<usize>,
    cond: std::sync::Condvar,
}

impl Semaphore {
    fn new(n: usize) -> Self {
        Self {
            state: Mutex::new(n),
            cond: std::sync::Condvar::new(),
        }
    }

    fn acquire(self: &Arc<Self>) -> Permit {
        let mut guard = self.state.lock().expect("sem poisoned");
        while *guard == 0 {
            guard = self.cond.wait(guard).expect("sem poisoned");
        }
        *guard -= 1;
        Permit {
            sem: Arc::clone(self),
        }
    }

    fn release(&self) {
        let mut guard = self.state.lock().expect("sem poisoned");
        *guard += 1;
        self.cond.notify_one();
    }
}

struct Permit {
    sem: Arc<Semaphore>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.sem.release();
    }
}

// Silence unused-import warnings when compiling without the loop sample.
#[allow(dead_code)]
fn _unused_timing(_: Instant) {}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, VecDeque};
    use std::fmt;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use orbit_agent::loop_engine::audit::{AuditSink, NullSink};
    use orbit_common::types::JobScheduleState;
    use orbit_common::types::activity_job::{
        ActivityV2, ActivityV2Spec, BackoffStrategy, BranchOutcome, DeterministicSpec, JobKind,
        JobV2, JobV2Step, JobV2StepBody, RetrySpec, TargetStep, V2ActivityCatalog, V2AuditEvent,
        V2AuditEventKind, load_job_asset,
    };
    use serde_json::{Value, json};
    use tracing::field::{Field, Visit};
    use tracing::{Event, Level, Metadata, Subscriber, span};

    use super::{
        DispatchError, V2AuditWriter, V2RuntimeHost, emit_job_event, execute_job, merge_job_input,
        resolve_job_catalog_refs_for_execution,
    };

    #[test]
    fn merges_object_defaults_with_explicit_object_input() {
        let defaults = json!({
            "mode": "pr",
            "base_branch": "agent-main",
            "max_tasks": 50,
            "max_bundle_size": 5
        });
        let explicit = json!({
            "base_branch": "main",
            "task_ids": ["T123"]
        });

        let merged = merge_job_input(Some(&defaults), &explicit);

        assert_eq!(
            merged,
            json!({
                "mode": "pr",
                "base_branch": "main",
                "max_tasks": 50,
                "max_bundle_size": 5,
                "task_ids": ["T123"]
            })
        );
    }

    #[test]
    fn preserves_non_object_explicit_input_without_merging() {
        let defaults = json!({
            "mode": "pr",
            "max_tasks": 50
        });
        let explicit = json!("override");

        let merged = merge_job_input(Some(&defaults), &explicit);

        assert_eq!(merged, explicit);
    }

    #[test]
    fn emit_job_event_dual_writes_step_lifecycle_to_audit_and_tracing() {
        let writer = test_writer("run-step-lifecycle");
        let captured = capture(|| {
            emit_job_event(
                &writer,
                Some("T-build"),
                V2AuditEventKind::StepStarted {
                    step_id: "build".to_string(),
                },
            )
            .expect("StepStarted emit");
            emit_job_event(
                &writer,
                Some("T-build"),
                V2AuditEventKind::StepFinished {
                    step_id: "build".to_string(),
                    outcome: "success".to_string(),
                },
            )
            .expect("StepFinished emit");
        });

        assert_eq!(
            captured.targets(),
            vec![
                ("orbit.job.step_started", Level::INFO),
                ("orbit.job.step_finished", Level::INFO),
            ]
        );
        // Field schema: job_run_id matches V2AuditWriter::run_id() and aligns
        // with the cli_runner producer's naming so JSONL consumers can join
        // job-step events with subprocess events on the same key.
        assert_eq!(
            captured.events[0].field("job_run_id"),
            Some("run-step-lifecycle")
        );
        assert_eq!(captured.events[0].field("task_id"), Some("T-build"));
        assert_eq!(captured.events[0].field("step_id"), Some("build"));
        // The legacy `run_id` field name is not emitted: prevent regression.
        assert_eq!(captured.events[0].field("run_id"), None);
        assert_eq!(captured.events[1].field("step_id"), Some("build"));
        assert_eq!(captured.events[1].field("outcome"), Some("success"));
        assert_eq!(captured.events[1].field("success"), Some("true"));
        assert_eq!(captured.events[1].field("task_id"), Some("T-build"));

        let snapshot = writer.events_snapshot().expect("audit snapshot");
        assert!(matches!(
            snapshot[0].kind,
            V2AuditEventKind::StepStarted { ref step_id } if step_id == "build"
        ));
        assert!(matches!(
            snapshot[1].kind,
            V2AuditEventKind::StepFinished { ref step_id, ref outcome }
                if step_id == "build" && outcome == "success"
        ));
    }

    #[test]
    fn emit_job_event_omits_task_id_when_none() {
        let writer = test_writer("run-no-task");
        let captured = capture(|| {
            emit_job_event(
                &writer,
                None,
                V2AuditEventKind::StepStarted {
                    step_id: "anon".to_string(),
                },
            )
            .expect("StepStarted emit");
        });

        assert_eq!(captured.events[0].field("job_run_id"), Some("run-no-task"));
        // None should NOT serialize a `task_id` field at all.
        assert_eq!(captured.events[0].field("task_id"), None);
    }

    #[test]
    fn emit_job_event_routes_step_finished_failure_to_error_level() {
        let writer = test_writer("run-fail");
        let captured = capture(|| {
            emit_job_event(
                &writer,
                None,
                V2AuditEventKind::StepFinished {
                    step_id: "deploy".to_string(),
                    outcome: "failed".to_string(),
                },
            )
            .expect("StepFinished failed emit");
        });

        assert_eq!(
            captured.targets(),
            vec![("orbit.job.step_finished", Level::ERROR)]
        );
        assert_eq!(captured.events[0].field("success"), Some("false"));
    }

    #[test]
    fn emit_job_event_uses_warn_for_retry_skip_no_converge_and_error_for_denied() {
        let writer = test_writer("run-warn-error");
        let captured = capture(|| {
            emit_job_event(
                &writer,
                None,
                V2AuditEventKind::StepRetry {
                    step_id: "flaky".to_string(),
                    attempt: 2,
                    next_backoff_ms: 250,
                },
            )
            .expect("StepRetry emit");
            emit_job_event(
                &writer,
                None,
                V2AuditEventKind::StepSkipped {
                    step_id: "flaky".to_string(),
                    reason: "when:false".to_string(),
                },
            )
            .expect("StepSkipped emit");
            emit_job_event(
                &writer,
                None,
                V2AuditEventKind::StepDenied {
                    step_id: "flaky".to_string(),
                    reason: "policy".to_string(),
                },
            )
            .expect("StepDenied emit");
            emit_job_event(
                &writer,
                None,
                V2AuditEventKind::LoopDidNotConverge {
                    step_id: "loopy".to_string(),
                    max_iterations: 5,
                },
            )
            .expect("LoopDidNotConverge emit");
        });

        assert_eq!(
            captured.targets(),
            vec![
                ("orbit.job.step_retry", Level::WARN),
                ("orbit.job.step_skipped", Level::WARN),
                ("orbit.job.step_denied", Level::ERROR),
                ("orbit.job.loop_did_not_converge", Level::WARN),
            ]
        );
        assert_eq!(captured.events[0].field("attempt"), Some("2"));
        assert_eq!(captured.events[0].field("next_backoff_ms"), Some("250"));
        assert_eq!(captured.events[1].field("reason"), Some("when:false"));
        assert_eq!(captured.events[3].field("max_iterations"), Some("5"));
    }

    #[test]
    fn emit_job_event_projects_fanout_loop_and_join_phases() {
        let writer = test_writer("run-fanout");
        let captured = capture(|| {
            emit_job_event(
                &writer,
                None,
                V2AuditEventKind::FanoutDispatched {
                    step_id: "scatter".to_string(),
                    worker_count: 3,
                },
            )
            .expect("FanoutDispatched emit");
            emit_job_event(
                &writer,
                None,
                V2AuditEventKind::WorkerState {
                    step_id: "scatter.worker".to_string(),
                    worker_index: 1,
                    state: "dispatched".to_string(),
                },
            )
            .expect("WorkerState emit");
            emit_job_event(
                &writer,
                None,
                V2AuditEventKind::FaninJoined {
                    step_id: "scatter".to_string(),
                    collected: 3,
                    failed: 0,
                },
            )
            .expect("FaninJoined emit");
            emit_job_event(
                &writer,
                None,
                V2AuditEventKind::StepJoin {
                    step_id: "merge".to_string(),
                    mode: "all".to_string(),
                    branch_outcomes: vec![
                        BranchOutcome {
                            branch_id: "a".to_string(),
                            outcome: "success".to_string(),
                        },
                        BranchOutcome {
                            branch_id: "b".to_string(),
                            outcome: "success".to_string(),
                        },
                    ],
                },
            )
            .expect("StepJoin emit");
            emit_job_event(
                &writer,
                None,
                V2AuditEventKind::LoopIterationStart {
                    step_id: "spin".to_string(),
                    iteration: 1,
                },
            )
            .expect("LoopIterationStart emit");
            emit_job_event(
                &writer,
                None,
                V2AuditEventKind::LoopIterationEnd {
                    step_id: "spin".to_string(),
                    iteration: 1,
                    broke: true,
                },
            )
            .expect("LoopIterationEnd emit");
        });

        assert_eq!(
            captured.targets(),
            vec![
                ("orbit.job.fanout", Level::INFO),
                ("orbit.job.worker_state", Level::INFO),
                ("orbit.job.fanout", Level::INFO),
                ("orbit.job.step_join", Level::INFO),
                ("orbit.job.loop_iteration", Level::INFO),
                ("orbit.job.loop_iteration", Level::INFO),
            ]
        );
        assert_eq!(captured.events[0].field("phase"), Some("dispatched"));
        assert_eq!(captured.events[0].field("worker_count"), Some("3"));
        assert_eq!(captured.events[1].field("worker_index"), Some("1"));
        assert_eq!(captured.events[1].field("state"), Some("dispatched"));
        assert_eq!(captured.events[2].field("phase"), Some("joined"));
        assert_eq!(captured.events[2].field("collected"), Some("3"));
        assert_eq!(captured.events[2].field("failed"), Some("0"));
        assert_eq!(captured.events[3].field("mode"), Some("all"));
        assert_eq!(captured.events[3].field("branch_count"), Some("2"));
        assert_eq!(captured.events[4].field("phase"), Some("start"));
        assert_eq!(captured.events[4].field("iteration"), Some("1"));
        assert_eq!(captured.events[5].field("phase"), Some("end"));
        assert_eq!(captured.events[5].field("broke"), Some("true"));
    }

    #[test]
    fn emit_job_event_audit_snapshot_matches_direct_emit_for_same_kinds() {
        // The dual-write helper must not perturb the audit-store representation.
        // Compare the snapshot from emit_job_event against a writer that calls
        // V2AuditWriter::emit directly with the same kinds in the same order.
        let kinds = || {
            vec![
                V2AuditEventKind::StepStarted {
                    step_id: "build".to_string(),
                },
                V2AuditEventKind::StepRetry {
                    step_id: "build".to_string(),
                    attempt: 1,
                    next_backoff_ms: 100,
                },
                V2AuditEventKind::StepFinished {
                    step_id: "build".to_string(),
                    outcome: "success".to_string(),
                },
            ]
        };

        let dual = test_writer("run-snapshot");
        for k in kinds() {
            emit_job_event(&dual, None, k).expect("dual emit");
        }
        let direct = test_writer("run-snapshot");
        for k in kinds() {
            direct.emit(k).expect("direct emit");
        }

        let dual_snapshot = dual.events_snapshot().expect("dual snapshot");
        let direct_snapshot = direct.events_snapshot().expect("direct snapshot");
        let dual_json = serde_json::to_value(&dual_snapshot).expect("dual json");
        let direct_json = serde_json::to_value(&direct_snapshot).expect("direct json");
        // The envelope timestamps and event_ids will differ; strip those before
        // comparing the body shape.
        let stripped = |value: serde_json::Value| -> serde_json::Value {
            let mut value = value;
            if let serde_json::Value::Array(items) = &mut value {
                for item in items {
                    if let serde_json::Value::Object(map) = item {
                        map.remove("ts");
                        map.remove("event_id");
                    }
                }
            }
            value
        };
        assert_eq!(stripped(dual_json), stripped(direct_json));
    }

    #[test]
    fn recovery_success_runs_one_post_recovery_attempt_with_exact_input_and_fs_profile() {
        let original_error = retryable_error("flaky", "dirty checkout");
        let host = RecoveryHost::new([
            (
                "flaky",
                vec![
                    Err(original_error.clone()),
                    Err(original_error.clone()),
                    Ok(json!({"fixed": true})),
                ],
            ),
            ("recover", vec![Ok(json!({"recovered": true}))]),
        ]);
        let job = recovery_job(Some("recover"), Some("wide"), "flaky", Some("narrow"), 2);
        let writer = std::sync::Arc::new(test_writer("run-recovery-success"));

        let outcome = execute_job(
            &job,
            Value::Null,
            "run-recovery-success",
            writer.clone(),
            &host,
        )
        .expect("job should recover");

        assert!(outcome.success);
        assert_eq!(host.actions(), vec!["flaky", "flaky", "recover", "flaky"]);
        assert_eq!(host.action_count("recover"), 1);
        assert_eq!(
            host.input_for_action("recover"),
            Some(json!({
                "failed_step_id": "build",
                "activity_name": "flaky",
                "error_message": original_error.to_string(),
                "attempt": 2,
                "max_attempts": 2
            }))
        );
        assert_eq!(
            host.fs_profile_for_action("recover"),
            Some(Some("narrow".to_string()))
        );

        let events = writer.events_snapshot().expect("audit snapshot");
        let recovery_events = recovery_events(&events);
        assert_eq!(recovery_events.len(), 1);
        assert!(matches!(
            recovery_events[0].kind,
            V2AuditEventKind::StepRecoveryAttempted {
                ref step_id,
                ref recovery_activity,
                recovery_succeeded: true,
            } if step_id == "build" && recovery_activity == "recover"
        ));
    }

    #[test]
    fn recovery_success_with_post_recovery_failure_returns_original_error_text() {
        let original_error = retryable_error("flaky", "first failure");
        let post_recovery_error = retryable_error("flaky", "post recovery still failing");
        let host = RecoveryHost::new([
            (
                "flaky",
                vec![
                    Err(original_error.clone()),
                    Err(original_error.clone()),
                    Err(post_recovery_error),
                ],
            ),
            ("recover", vec![Ok(json!({"recovered": true}))]),
        ]);
        let job = recovery_job(Some("recover"), None, "flaky", None, 2);
        let writer = std::sync::Arc::new(test_writer("run-post-recovery-failure"));

        let err = execute_job(
            &job,
            Value::Null,
            "run-post-recovery-failure",
            writer.clone(),
            &host,
        )
        .expect_err("post-recovery failure should surface original error");

        assert_eq!(err.to_string(), original_error.to_string());
        assert_eq!(host.action_count("recover"), 1);
        assert_eq!(recovery_events(&writer.events_snapshot().unwrap()).len(), 1);
    }

    #[test]
    fn recovery_activity_error_returns_original_error_text() {
        let original_error = retryable_error("flaky", "precondition failed");
        let host = RecoveryHost::new([
            ("flaky", vec![Err(original_error.clone())]),
            (
                "recover",
                vec![Err(retryable_error("recover", "could not fix"))],
            ),
        ]);
        let job = recovery_job(Some("recover"), None, "flaky", None, 1);
        let writer = std::sync::Arc::new(test_writer("run-recovery-error"));

        let err = execute_job(
            &job,
            Value::Null,
            "run-recovery-error",
            writer.clone(),
            &host,
        )
        .expect_err("recovery error should surface original error");

        assert_eq!(err.to_string(), original_error.to_string());
        assert_eq!(host.action_count("recover"), 1);
        let events = writer.events_snapshot().expect("audit snapshot");
        assert!(matches!(
            recovery_events(&events)[0].kind,
            V2AuditEventKind::StepRecoveryAttempted {
                recovery_succeeded: false,
                ..
            }
        ));
    }

    #[test]
    fn non_retryable_failure_skips_recovery_and_audit_event() {
        let host = RecoveryHost::new([
            (
                "flaky",
                vec![Err(DispatchError::ToolDenied {
                    tool_name: "fs.write".to_string(),
                    iteration: 1,
                })],
            ),
            ("recover", vec![Ok(json!({"recovered": true}))]),
        ]);
        let job = recovery_job(Some("recover"), None, "flaky", None, 2);
        let writer = std::sync::Arc::new(test_writer("run-non-retryable"));

        let err = execute_job(
            &job,
            Value::Null,
            "run-non-retryable",
            writer.clone(),
            &host,
        )
        .expect_err("tool denial should bypass recovery");

        assert!(matches!(err, DispatchError::ToolDenied { .. }));
        assert_eq!(host.action_count("recover"), 0);
        assert!(recovery_events(&writer.events_snapshot().unwrap()).is_empty());
    }

    #[test]
    fn no_recovery_activity_preserves_success_and_failure_paths() {
        let original_error = retryable_error("flaky", "still failing");
        let failing_host = RecoveryHost::new([("flaky", vec![Err(original_error.clone())])]);
        let failing_job = recovery_job(None, None, "flaky", None, 1);
        let failing_writer = std::sync::Arc::new(test_writer("run-no-recovery-failure"));

        let err = execute_job(
            &failing_job,
            Value::Null,
            "run-no-recovery-failure",
            failing_writer.clone(),
            &failing_host,
        )
        .expect_err("retryable failure should remain the original error");

        assert_eq!(err.to_string(), original_error.to_string());
        assert!(recovery_events(&failing_writer.events_snapshot().unwrap()).is_empty());

        let success_host = RecoveryHost::new([("stable", vec![Ok(json!({"ok": true}))])]);
        let success_job = recovery_job(None, None, "stable", None, 1);
        let success_writer = std::sync::Arc::new(test_writer("run-no-recovery-success"));

        let outcome = execute_job(
            &success_job,
            Value::Null,
            "run-no-recovery-success",
            success_writer.clone(),
            &success_host,
        )
        .expect("success path should remain unchanged");

        assert!(outcome.success);
        assert!(recovery_events(&success_writer.events_snapshot().unwrap()).is_empty());
    }

    #[test]
    fn unknown_recovery_activity_name_is_job_validation_during_catalog_resolution() {
        let yaml = r#"
schemaVersion: 2
kind: Job
metadata:
  name: missing_recovery
spec:
  state: enabled
  recovery_activity: missing
  steps:
    - id: build
      spec:
        type: deterministic
        action: flaky
"#;
        let mut job = load_job_asset(yaml).expect("job yaml").spec;
        let catalog = V2ActivityCatalog::new();

        let err = resolve_job_catalog_refs_for_execution(&mut job, &catalog)
            .expect_err("missing recovery activity should fail resolution");

        assert!(matches!(
            err,
            DispatchError::JobValidation(ref message)
                if message.contains("recovery_activity `missing` not found")
        ));
    }

    fn recovery_job(
        recovery_name: Option<&str>,
        recovery_fs_profile: Option<&str>,
        step_action: &str,
        step_fs_profile: Option<&str>,
        max_attempts: u32,
    ) -> JobV2 {
        JobV2 {
            state: JobScheduleState::Enabled,
            default_input: None,
            recovery_activity: recovery_name.map(str::to_string),
            resolved_recovery_activity: recovery_name
                .map(|name| deterministic_activity(name, recovery_fs_profile)),
            max_active_runs: 1,
            kind: JobKind::Workflow,
            steps: vec![JobV2Step {
                id: "build".to_string(),
                when: None,
                retry: Some(RetrySpec {
                    max_attempts,
                    initial_backoff_ms: 0,
                    backoff_cap_ms: 0,
                    backoff_strategy: BackoffStrategy::Linear,
                }),
                body: JobV2StepBody::Target(TargetStep {
                    spec: deterministic_activity(step_action, None).spec,
                    activity_name: None,
                    fs_profile: step_fs_profile.map(str::to_string),
                    default_input: None,
                    timeout_seconds: 0,
                    session: None,
                }),
            }],
        }
    }

    fn deterministic_activity(action: &str, fs_profile: Option<&str>) -> ActivityV2 {
        ActivityV2 {
            description: format!("deterministic {action}"),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            fs_profile: fs_profile.map(str::to_string),
            spec: ActivityV2Spec::Deterministic(DeterministicSpec {
                action: action.to_string(),
                config: Value::Null,
            }),
        }
    }

    fn retryable_error(action: &str, message: &str) -> DispatchError {
        DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: message.to_string(),
        }
    }

    fn recovery_events(events: &[V2AuditEvent]) -> Vec<&V2AuditEvent> {
        events
            .iter()
            .filter(|event| matches!(event.kind, V2AuditEventKind::StepRecoveryAttempted { .. }))
            .collect()
    }

    #[derive(Debug, Clone)]
    struct DeterministicCall {
        action: String,
        input: Value,
        fs_profile: Option<String>,
    }

    struct RecoveryHost {
        responses: StdMutex<HashMap<String, VecDeque<Result<Value, DispatchError>>>>,
        calls: StdMutex<Vec<DeterministicCall>>,
        pending_fs_profiles: StdMutex<VecDeque<Option<String>>>,
    }

    impl RecoveryHost {
        fn new<const N: usize>(responses: [(&str, Vec<Result<Value, DispatchError>>); N]) -> Self {
            Self {
                responses: StdMutex::new(
                    responses
                        .into_iter()
                        .map(|(action, outcomes)| {
                            (action.to_string(), outcomes.into_iter().collect())
                        })
                        .collect(),
                ),
                calls: StdMutex::new(Vec::new()),
                pending_fs_profiles: StdMutex::new(VecDeque::new()),
            }
        }

        fn actions(&self) -> Vec<String> {
            self.calls
                .lock()
                .expect("calls lock")
                .iter()
                .map(|call| call.action.clone())
                .collect()
        }

        fn action_count(&self, action: &str) -> usize {
            self.calls
                .lock()
                .expect("calls lock")
                .iter()
                .filter(|call| call.action == action)
                .count()
        }

        fn input_for_action(&self, action: &str) -> Option<Value> {
            self.calls
                .lock()
                .expect("calls lock")
                .iter()
                .find(|call| call.action == action)
                .map(|call| call.input.clone())
        }

        fn fs_profile_for_action(&self, action: &str) -> Option<Option<String>> {
            self.calls
                .lock()
                .expect("calls lock")
                .iter()
                .find(|call| call.action == action)
                .map(|call| call.fs_profile.clone())
        }
    }

    impl V2RuntimeHost for RecoveryHost {
        fn run_deterministic(
            &self,
            action: &str,
            _config: &Value,
            input: &Value,
            _tool_context: orbit_tools::ToolContext,
        ) -> Result<Value, DispatchError> {
            let fs_profile = self
                .pending_fs_profiles
                .lock()
                .expect("fs profiles lock")
                .pop_front()
                .unwrap_or(None);
            self.calls
                .lock()
                .expect("calls lock")
                .push(DeterministicCall {
                    action: action.to_string(),
                    input: input.clone(),
                    fs_profile,
                });

            self.responses
                .lock()
                .expect("responses lock")
                .get_mut(action)
                .and_then(VecDeque::pop_front)
                .unwrap_or_else(|| Ok(json!({"action": action})))
        }

        fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
            Err(DispatchError::AgentLoopFailed(
                "test host: no credentials".into(),
            ))
        }

        fn resolve_cli_executor(
            &self,
            _provider: &str,
        ) -> Result<super::super::dispatcher::ResolvedCliExecutor, DispatchError> {
            Err(DispatchError::CliInvocationFailed(
                "test host: no CLI mapping".into(),
            ))
        }

        fn tool_context_for_activity(
            &self,
            fs_profile: Option<&str>,
            _fs_audit: Option<std::sync::Arc<dyn orbit_tools::FsAuditLogger>>,
        ) -> orbit_tools::ToolContext {
            self.pending_fs_profiles
                .lock()
                .expect("fs profiles lock")
                .push_back(fs_profile.map(str::to_string));
            orbit_tools::ToolContext::default()
        }
    }

    fn test_writer(run_id: &str) -> V2AuditWriter {
        let inner: std::sync::Arc<dyn AuditSink> = std::sync::Arc::new(NullSink);
        V2AuditWriter::new(run_id, "test-agent", inner)
    }

    fn capture<F>(f: F) -> CapturedTrace
    where
        F: FnOnce(),
    {
        let events = std::sync::Arc::new(StdMutex::new(Vec::<CapturedEvent>::new()));
        let subscriber = CaptureSubscriber {
            events: events.clone(),
            next_span_id: AtomicU64::new(1),
        };
        let dispatch = tracing::Dispatch::new(subscriber);
        tracing::dispatcher::with_default(&dispatch, f);
        CapturedTrace {
            events: events.lock().expect("events lock").clone(),
        }
    }

    struct CapturedTrace {
        events: Vec<CapturedEvent>,
    }

    impl CapturedTrace {
        fn targets(&self) -> Vec<(&str, Level)> {
            self.events
                .iter()
                .map(|e| (e.target.as_str(), e.level))
                .collect()
        }
    }

    #[derive(Debug, Clone)]
    struct CapturedEvent {
        target: String,
        level: Level,
        fields: BTreeMap<String, String>,
    }

    impl CapturedEvent {
        fn field(&self, name: &str) -> Option<&str> {
            self.fields.get(name).map(String::as_str)
        }
    }

    struct CaptureSubscriber {
        events: std::sync::Arc<StdMutex<Vec<CapturedEvent>>>,
        next_span_id: AtomicU64,
    }

    impl Subscriber for CaptureSubscriber {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &span::Attributes<'_>) -> span::Id {
            span::Id::from_u64(self.next_span_id.fetch_add(1, Ordering::Relaxed))
        }

        fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}

        fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut visitor = FieldCapture::default();
            event.record(&mut visitor);
            let metadata = event.metadata();
            self.events
                .lock()
                .expect("events lock")
                .push(CapturedEvent {
                    target: metadata.target().to_string(),
                    level: *metadata.level(),
                    fields: visitor.fields,
                });
        }

        fn enter(&self, _span: &span::Id) {}
        fn exit(&self, _span: &span::Id) {}
    }

    #[derive(Default)]
    struct FieldCapture {
        fields: BTreeMap<String, String>,
    }

    impl Visit for FieldCapture {
        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_bool(&mut self, field: &Field, value: bool) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.fields
                .insert(field.name().to_string(), format!("{value:?}"));
        }
    }
}
