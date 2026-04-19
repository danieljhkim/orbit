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
use orbit_types::v2::{
    ActivityV2Spec, AgentLoopSpec, BackoffStrategy, BranchOutcome, FanInSpec, FanOutBlock, JobV2,
    JobV2Step, JobV2StepBody, JoinMode, LoopBlock, ParallelBlock, RetrySpec, TargetStep,
    V2AuditEventKind,
};
use serde_json::Value;

use crate::job_runner::evaluate_bool_expr;
use crate::template::{self, TemplateContext};

use super::agent_loop_driver::drive_agent_loop_with_session;
use super::audit_writer::V2AuditWriter;
use super::dispatcher::{DispatchError, V2DispatchInput, V2RuntimeHost, dispatch_v2_activity};

const DEFAULT_MODEL_FOR_SESSION: &str = "claude-sonnet-4-5";

/// Result of executing a v2 Job end-to-end.
#[derive(Debug, Clone)]
pub struct JobOutcome {
    pub success: bool,
    pub pipeline: Value,
    pub message: Option<String>,
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

    let base_input = match (&job.default_input, &input) {
        (Some(d), Value::Null) => d.clone(),
        _ => input.clone(),
    };

    let ctx = ExecCtx {
        run_id: run_id.to_string(),
        audit: audit.clone(),
        host,
        input: base_input.clone(),
        pipeline: Mutex::new(HashMap::new()),
        sessions: Mutex::new(HashMap::new()),
        item: None,
    };

    let mut overall_ok = true;
    for step in &job.steps {
        let outcome = run_step(step, &ctx)?;
        if !outcome.success {
            overall_ok = false;
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
        message: None,
    })
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
    pipeline: Mutex<HashMap<String, Value>>,
    sessions: Mutex<HashMap<String, Session>>,
    /// `Some(value)` inside a fan-out worker. Rendered into template context
    /// as `{{ item }}`.
    item: Option<Value>,
}

impl ExecCtx<'_> {
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
        TemplateContext {
            input,
            env: Default::default(),
            workspace_path: None,
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
            let _ = ctx.audit.emit(V2AuditEventKind::StepSkipped {
                step_id: step.id.clone(),
                reason: format!("when:{expr} => false"),
            });
            return Ok(StepOutcome {
                success: true,
                output: Value::Null,
                skipped: true,
            });
        }
    }

    let step_event_id = ctx
        .audit
        .emit(V2AuditEventKind::StepStarted {
            step_id: step.id.clone(),
        })
        .map_err(|e| DispatchError::AuditFailed(format!("{e:?}")))?;
    let _ = ctx.audit.push_parent(step_event_id);

    let result = run_step_with_retry(step, ctx);

    let _ = ctx.audit.pop_parent();
    let outcome_str = match &result {
        Ok(StepOutcome { success: true, .. }) => "success",
        Ok(_) => "failed",
        Err(_) => "error",
    };
    let _ = ctx.audit.emit(V2AuditEventKind::StepFinished {
        step_id: step.id.clone(),
        outcome: outcome_str.to_string(),
    });

    result
}

fn run_step_with_retry(step: &JobV2Step, ctx: &ExecCtx<'_>) -> Result<StepOutcome, DispatchError> {
    let Some(retry) = &step.retry else {
        // No retry wrapper — single attempt.
        return match run_step_body(step, ctx) {
            Ok(outcome) => Ok(outcome),
            Err(err) if err.is_non_retryable() => {
                emit_denied_if_applicable(&err, &step.id, &ctx.audit);
                Err(err)
            }
            Err(err) => Err(err),
        };
    };

    let mut last_err: Option<DispatchError> = None;
    for attempt in 0..retry.max_attempts.max(1) {
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
                emit_denied_if_applicable(&err, &step.id, &ctx.audit);
                return Err(err);
            }
            Err(err) => {
                last_err = Some(err);
            }
        }
        if attempt + 1 >= retry.max_attempts {
            break;
        }
        let backoff_ms = compute_backoff_ms(retry, attempt);
        let _ = ctx.audit.emit(V2AuditEventKind::StepRetry {
            step_id: step.id.clone(),
            attempt: attempt + 1,
            next_backoff_ms: backoff_ms,
        });
        thread::sleep(Duration::from_millis(backoff_ms));
    }

    match last_err {
        Some(err) => Err(err),
        None => Ok(StepOutcome {
            success: false,
            output: Value::Null,
            skipped: false,
        }),
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

fn emit_denied_if_applicable(err: &DispatchError, step_id: &str, audit: &Arc<V2AuditWriter>) {
    if matches!(err, DispatchError::ToolDenied { .. }) {
        let _ = audit.emit(V2AuditEventKind::StepDenied {
            step_id: step_id.to_string(),
            reason: err.to_string(),
        });
    }
}

fn run_step_body(step: &JobV2Step, ctx: &ExecCtx<'_>) -> Result<StepOutcome, DispatchError> {
    match &step.body {
        JobV2StepBody::Target(t) => run_target(step, t, ctx),
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
            if agent_spec.backend != orbit_types::v2::Backend::Http {
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
                input: rendered_input,
                audit: ctx.audit.clone(),
                run_id: &ctx.run_id,
                host: Some(ctx.host),
            })?;
            let out = dispatch.output.clone();
            record_pipeline(ctx, &step.id, out.clone());
            Ok(StepOutcome {
                success: dispatch.success,
                output: out,
                skipped: false,
            })
        }
        _ => {
            // Deterministic / Shell dispatch via the existing Phase 2 entry.
            let dispatch = dispatch_v2_activity(V2DispatchInput {
                activity_name: &step.id,
                spec: &t.spec,
                input: rendered_input,
                audit: ctx.audit.clone(),
                run_id: &ctx.run_id,
                host: Some(ctx.host),
            })?;
            let out = dispatch.output.clone();
            record_pipeline(ctx, &step.id, out.clone());
            Ok(StepOutcome {
                success: dispatch.success,
                output: out,
                skipped: false,
            })
        }
    }
}

fn run_agent_loop_outcome(
    step: &JobV2Step,
    spec: &AgentLoopSpec,
    api_key: Option<&str>,
    session: &mut Session,
    input: &Value,
    ctx: &ExecCtx<'_>,
) -> Result<StepOutcome, DispatchError> {
    let outcome = drive_agent_loop_with_session(
        spec,
        api_key,
        &ctx.run_id,
        ctx.audit.clone(),
        session,
        input,
    )?;
    let out_json = serde_json::json!({
        "final_message": outcome.final_message,
        "terminate_reason": format!("{:?}", outcome.terminate_reason),
    });
    record_pipeline(ctx, &step.id, out_json.clone());
    Ok(StepOutcome {
        success: true,
        output: out_json,
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
    let _ = ctx.audit.emit(V2AuditEventKind::StepJoin {
        step_id: step.id.clone(),
        mode: mode_label.to_string(),
        branch_outcomes: outcomes.clone(),
    });

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
    let items_rendered = template::render(&block.items, &tctx)
        .map_err(|err| DispatchError::JobExecution(format!("fan_out.items render: {err}")))?;
    let items: Vec<Value> = serde_json::from_str(&items_rendered).unwrap_or_else(|_| {
        // Fallback: split CSV-style on whitespace/commas if the rendered
        // value isn't a JSON array (handles simple shell-produced outputs).
        items_rendered
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(|s| Value::String(s.to_string()))
            .collect()
    });
    let worker_count = items.len() as u32;

    let _ = ctx.audit.emit(V2AuditEventKind::FanoutDispatched {
        step_id: step.id.clone(),
        worker_count,
    });

    if items.is_empty() {
        let _ = ctx.audit.emit(V2AuditEventKind::FaninJoined {
            step_id: step.id.clone(),
            collected: 0,
            failed: 0,
        });
        return Ok(StepOutcome {
            success: true,
            output: Value::Array(Vec::new()),
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
                let _ = audit.emit(V2AuditEventKind::WorkerState {
                    step_id: worker_step.id.clone(),
                    worker_index: idx,
                    state: "dispatched".to_string(),
                });
                let worker_ctx = ExecCtx {
                    run_id,
                    audit: audit.clone(),
                    host,
                    input: base_input,
                    pipeline: Mutex::new(pipeline_snapshot),
                    sessions: Mutex::new(HashMap::new()),
                    item: Some(item),
                };
                let res = run_step(&worker_step, &worker_ctx);
                let state = match &res {
                    Ok(o) if o.success => "finished",
                    Ok(_) => "failed",
                    Err(_) => "failed",
                };
                let _ = audit.emit(V2AuditEventKind::WorkerState {
                    step_id: worker_step.id.clone(),
                    worker_index: idx,
                    state: state.to_string(),
                });
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

    let _ = ctx.audit.emit(V2AuditEventKind::FaninJoined {
        step_id: step.id.clone(),
        collected: collected_count,
        failed: failed_count,
    });

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
    let mut broke = false;
    let mut last_iter: u32 = 0;
    for iter in 1..=block.max_iterations {
        last_iter = iter;
        let _ = ctx.audit.emit(V2AuditEventKind::LoopIterationStart {
            step_id: step.id.clone(),
            iteration: iter,
        });

        for body in &block.steps {
            let outcome = run_step(body, ctx)?;
            if !outcome.success {
                return Ok(StepOutcome {
                    success: false,
                    output: outcome.output,
                    skipped: false,
                });
            }
        }

        // Evaluate break_when after the body runs so the body can populate
        // pipeline state that the expression references.
        let should_break = if let Some(expr) = &block.break_when {
            let tctx = ctx.template_ctx();
            evaluate_bool_expr(expr, &tctx)
                .map_err(|err| DispatchError::JobExecution(format!("break_when: {err}")))?
        } else {
            false
        };

        let _ = ctx.audit.emit(V2AuditEventKind::LoopIterationEnd {
            step_id: step.id.clone(),
            iteration: iter,
            broke: should_break,
        });

        if should_break {
            broke = true;
            break;
        }
    }

    if !broke && block.break_when.is_some() {
        let _ = ctx.audit.emit(V2AuditEventKind::LoopDidNotConverge {
            step_id: step.id.clone(),
            max_iterations: block.max_iterations,
        });
    }

    let out = serde_json::json!({
        "iterations": last_iter,
        "broke": broke,
    });
    record_pipeline(ctx, &step.id, out.clone());

    Ok(StepOutcome {
        success: true,
        output: out,
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
