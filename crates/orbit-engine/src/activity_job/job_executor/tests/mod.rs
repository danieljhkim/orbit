#![allow(missing_docs)]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use orbit_agent::loop_engine::audit::{AuditSink, NullSink};
use orbit_common::types::JobScheduleState;
use orbit_common::types::activity_job::{
    ActivityV2, ActivityV2Spec, BackoffStrategy, BranchOutcome, DeterministicSpec, FanInSpec,
    FanOutBlock, JobKind, JobV2, JobV2Step, JobV2StepBody, JoinMode, LoopBlock, ParallelBlock,
    RetrySpec, TargetStep, V2ActivityCatalog, V2AuditEvent, V2AuditEventKind, load_job_asset,
};
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber, span};

use super::*;

mod audit;
mod fanout;
mod r#loop;
mod parallel;
mod pipeline_durability;
mod recovery;
mod step;
mod target;
mod templating;

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

// --------------------------------------------------------------------------
// Shared scripted host for executor-block tests
// --------------------------------------------------------------------------
//
// `ScriptedHost` is a minimal `V2RuntimeHost` returning scripted outcomes
// per deterministic-action name. Per ADR-047 each executor-block test module
// reuses this scaffolding instead of re-deriving its own; broadening the
// surface (e.g. agent_loop or shell hosts) belongs here rather than in any
// individual test module.

/// Outcome a `ScriptedHost` returns for a particular call. `SleepOk` lets a
/// worker hold a permit long enough for max-concurrency tests to observe
/// overlap without flaky wall-clock timing.
#[derive(Clone)]
pub(super) enum Action {
    Ok(Value),
    Err(DispatchError),
    SleepOk { ms: u64, value: Value },
    SleepInputMsThenEcho { ms_field: &'static str },
}

pub(super) struct ScriptedHost {
    responses: StdMutex<HashMap<String, VecDeque<Action>>>,
    call_log: StdMutex<Vec<String>>,
    in_flight: AtomicUsize,
    peak_in_flight: AtomicUsize,
}

impl ScriptedHost {
    pub(super) fn new<const N: usize>(actions: [(&str, Vec<Action>); N]) -> Self {
        Self {
            responses: StdMutex::new(
                actions
                    .into_iter()
                    .map(|(name, queue)| (name.to_string(), queue.into_iter().collect()))
                    .collect(),
            ),
            call_log: StdMutex::new(Vec::new()),
            in_flight: AtomicUsize::new(0),
            peak_in_flight: AtomicUsize::new(0),
        }
    }

    pub(super) fn call_count(&self, action: &str) -> usize {
        self.call_log
            .lock()
            .expect("call log")
            .iter()
            .filter(|name| *name == action)
            .count()
    }

    pub(super) fn peak_in_flight(&self) -> usize {
        self.peak_in_flight.load(Ordering::SeqCst)
    }
}

impl V2RuntimeHost for ScriptedHost {
    fn run_deterministic(
        &self,
        action: &str,
        _config: &Value,
        input: &Value,
        _tool_context: orbit_tools::ToolContext,
    ) -> Result<Value, DispatchError> {
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        let mut peak = self.peak_in_flight.load(Ordering::SeqCst);
        while now > peak {
            match self.peak_in_flight.compare_exchange(
                peak,
                now,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(observed) => peak = observed,
            }
        }
        self.call_log
            .lock()
            .expect("call log")
            .push(action.to_string());
        let next = self
            .responses
            .lock()
            .expect("responses")
            .get_mut(action)
            .and_then(VecDeque::pop_front);
        let result = match next {
            Some(Action::Ok(value)) => Ok(value),
            Some(Action::Err(err)) => Err(err),
            Some(Action::SleepOk { ms, value }) => {
                std::thread::sleep(Duration::from_millis(ms));
                Ok(value)
            }
            Some(Action::SleepInputMsThenEcho { ms_field }) => {
                let ms = input.get(ms_field).and_then(Value::as_u64).ok_or_else(|| {
                    DispatchError::JobExecution(format!(
                        "scripted host missing numeric sleep field `{ms_field}`"
                    ))
                })?;
                std::thread::sleep(Duration::from_millis(ms));
                Ok(input.clone())
            }
            // Default: succeed with `{ "action": <name> }` so untyped tests
            // don't have to script every call.
            None => Ok(json!({ "action": action })),
        };
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        result
    }

    fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::AgentLoopFailed(
            "scripted host: no credentials".into(),
        ))
    }

    fn resolve_cli_executor(
        &self,
        _provider: &str,
    ) -> Result<super::super::dispatcher::ResolvedCliExecutor, DispatchError> {
        Err(DispatchError::CliInvocationFailed(
            "scripted host: no CLI mapping".into(),
        ))
    }

    fn tool_context_for_activity(
        &self,
        _run_id: Option<&str>,
        _fs_profile: Option<&str>,
        _fs_audit: Option<std::sync::Arc<dyn orbit_tools::FsAuditLogger>>,
    ) -> orbit_tools::ToolContext {
        orbit_tools::ToolContext::default()
    }
}

// --------------------------------------------------------------------------
// Job/step builders
// --------------------------------------------------------------------------

pub(super) fn deterministic_target(action: &str) -> TargetStep {
    TargetStep {
        spec: ActivityV2Spec::Deterministic(DeterministicSpec {
            action: action.to_string(),
            config: Value::Null,
        }),
        activity_name: None,
        fs_profile: None,
        default_input: None,
        timeout_seconds: 0,
        session: None,
        role: None,
    }
}

pub(super) fn target_step(id: &str, action: &str) -> JobV2Step {
    JobV2Step {
        id: id.to_string(),
        when: None,
        retry: None,
        recovery_activity: None,
        resolved_recovery_activity: None,
        body: JobV2StepBody::Target(deterministic_target(action)),
    }
}

pub(super) fn target_step_with_retry(id: &str, action: &str, max_attempts: u32) -> JobV2Step {
    JobV2Step {
        id: id.to_string(),
        when: None,
        retry: Some(RetrySpec {
            max_attempts,
            initial_backoff_ms: 0,
            backoff_cap_ms: 0,
            backoff_strategy: BackoffStrategy::Linear,
        }),
        recovery_activity: None,
        resolved_recovery_activity: None,
        body: JobV2StepBody::Target(deterministic_target(action)),
    }
}

pub(super) fn job_with_steps(steps: Vec<JobV2Step>) -> JobV2 {
    JobV2 {
        state: JobScheduleState::Enabled,
        default_input: None,
        recovery_activity: None,
        resolved_recovery_activity: None,
        max_active_runs: 1,
        kind: JobKind::Workflow,
        steps,
    }
}

pub(super) fn parallel_step(id: &str, mode: JoinMode, branches: Vec<JobV2Step>) -> JobV2Step {
    JobV2Step {
        id: id.to_string(),
        when: None,
        retry: None,
        recovery_activity: None,
        resolved_recovery_activity: None,
        body: JobV2StepBody::Parallel {
            parallel: ParallelBlock {
                join: mode,
                branches,
            },
        },
    }
}

pub(super) fn fanout_step(
    id: &str,
    items_expr: &str,
    max_workers: u32,
    worker: JobV2Step,
    join: JoinMode,
    collect: Option<&str>,
) -> JobV2Step {
    JobV2Step {
        id: id.to_string(),
        when: None,
        retry: None,
        recovery_activity: None,
        resolved_recovery_activity: None,
        body: JobV2StepBody::FanOut {
            fan_out: FanOutBlock {
                items: items_expr.to_string(),
                max_workers,
                worker: Box::new(worker),
            },
            fan_in: FanInSpec {
                join,
                collect: collect.map(str::to_string),
            },
        },
    }
}

pub(super) fn loop_step(
    id: &str,
    items_expr: Option<&str>,
    max_iterations: u32,
    break_when: Option<&str>,
    body: Vec<JobV2Step>,
) -> JobV2Step {
    JobV2Step {
        id: id.to_string(),
        when: None,
        retry: None,
        recovery_activity: None,
        resolved_recovery_activity: None,
        body: JobV2StepBody::Loop {
            loop_: LoopBlock {
                items: items_expr.map(str::to_string),
                max_iterations,
                break_when: break_when.map(str::to_string),
                steps: body,
            },
        },
    }
}

pub(super) fn run_job(host: &ScriptedHost, job: &JobV2, input: Value, run_id: &str) -> JobOutcome {
    let writer = std::sync::Arc::new(test_writer(run_id));
    execute_job(job, input, run_id, writer, host).expect("execute_job ok")
}
