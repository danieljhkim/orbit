use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering};

use orbit_agent::loop_engine::audit::{AuditSink, NullSink};
use orbit_common::types::JobScheduleState;
use orbit_common::types::activity_job::{
    ActivityV2, ActivityV2Spec, BackoffStrategy, BranchOutcome, DeterministicSpec, JobKind, JobV2,
    JobV2Step, JobV2StepBody, RetrySpec, TargetStep, V2ActivityCatalog, V2AuditEvent,
    V2AuditEventKind, load_job_asset,
};
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber, span};

use super::*;

mod audit_tests;
mod recovery_tests;
mod target_tests;

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
