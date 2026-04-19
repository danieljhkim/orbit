use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use chrono::Utc;
use orbit_agent::loop_engine::InMemorySink;
use orbit_agent::loop_engine::audit::{AuditSink, LoopAuditEvent};
use orbit_types::v2::{
    AUDIT_ENVELOPE_SCHEMA_VERSION, V2AuditEnvelope, V2AuditEvent, V2AuditEventKind,
};
use thiserror::Error;

use super::jsonl_sink::V2JsonlSink;

/// Writes §7 v2 audit envelope events. Nests the existing loop-engine events
/// underneath an Activity event via `parent_event_id` so the whole tree
/// (Run → Step → Activity → http.*/tool.call.*) is traversable by ID.
///
/// This writer owns the run_id / agent_identity context and emits events both
/// as structured JSON (for orbit-audit consumers) and as an inner loop sink
/// passthrough (so loop-level http.* and tool.call.* events continue to flow
/// through the existing JSONL path).
pub struct V2AuditWriter {
    run_id: String,
    agent_identity: String,
    workspace_path: Option<String>,
    inner: Arc<dyn AuditSink>,
    envelope_sink: Option<Arc<V2JsonlSink>>,
    events: Mutex<Vec<V2AuditEvent>>,
    event_counter: Mutex<u64>,
    parent_stacks: Mutex<HashMap<ThreadId, Vec<String>>>,
}

/// Restores the calling thread's previous parent stack on drop.
pub(crate) struct ParentStackGuard<'a> {
    writer: &'a V2AuditWriter,
    thread_id: ThreadId,
    previous: Option<Vec<String>>,
}

#[derive(Debug, Error)]
pub enum WriteError {
    #[error("audit writer mutex poisoned")]
    Poisoned,
}

impl V2AuditWriter {
    pub fn new(
        run_id: impl Into<String>,
        agent_identity: impl Into<String>,
        inner: Arc<dyn AuditSink>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            agent_identity: agent_identity.into(),
            workspace_path: None,
            inner,
            envelope_sink: None,
            events: Mutex::new(Vec::new()),
            event_counter: Mutex::new(0),
            parent_stacks: Mutex::new(HashMap::new()),
        }
    }

    /// Attach a JSONL sink for §7 envelope events. When set, every emitted
    /// envelope event is persisted to disk alongside the in-memory snapshot
    /// (resolves Phase 2a design-risk #2).
    pub fn with_envelope_sink(mut self, sink: Arc<V2JsonlSink>) -> Self {
        self.envelope_sink = Some(sink);
        self
    }

    /// Attach the originating workspace path for §7 `workspace_path`
    /// provenance. Call before the writer is shared (`Arc::new`). Absent
    /// when the caller has no meaningful workspace (stub hosts, smokes).
    pub fn with_workspace_path(mut self, path: impl Into<String>) -> Self {
        self.workspace_path = Some(path.into());
        self
    }

    /// High-level constructor for CLI / library callers that don't want to
    /// name the loop-level sink types directly (orbit-core's primary use
    /// case). Creates an `InMemorySink` backed by `audit_root/blobs/` and a
    /// `V2JsonlSink` at `audit_root/v2_loop/{run_id}.jsonl`, wires them
    /// together, and returns a ready-to-dispatch writer.
    ///
    /// Callers that need a custom sink configuration use `new` +
    /// `with_envelope_sink` directly.
    pub fn with_disk_sinks(
        audit_root: &Path,
        run_id: impl Into<String>,
        agent_identity: impl Into<String>,
        workspace_path: Option<&Path>,
    ) -> std::io::Result<Arc<Self>> {
        let run_id = run_id.into();
        let blob_dir = audit_root.join("blobs");
        std::fs::create_dir_all(&blob_dir)?;
        let inner: Arc<dyn AuditSink> = Arc::new(InMemorySink::new(blob_dir));
        let envelope_sink = Arc::new(V2JsonlSink::open(audit_root, &run_id)?);
        let mut writer = Self::new(run_id, agent_identity, inner).with_envelope_sink(envelope_sink);
        if let Some(path) = workspace_path {
            writer = writer.with_workspace_path(path.display().to_string());
        }
        Ok(Arc::new(writer))
    }

    /// Path to the JSONL sink's log file, if one is attached. Used by CLI
    /// callers to report where envelope events were persisted.
    pub fn envelope_log_path(&self) -> Option<std::path::PathBuf> {
        self.envelope_sink
            .as_ref()
            .map(|s| s.log_path().to_path_buf())
    }

    /// Emit a v2 envelope event of the given kind. Returns the event_id so
    /// callers can use it as a parent for nested events.
    pub fn emit(&self, kind: V2AuditEventKind) -> Result<String, WriteError> {
        let event_id = self.next_event_id()?;
        let parent_event_id = self.current_parent_event_id()?;
        let event_type = event_type_of(&kind).to_string();
        let envelope = V2AuditEnvelope {
            schema_version: AUDIT_ENVELOPE_SCHEMA_VERSION,
            event_type,
            event_id: event_id.clone(),
            ts: Utc::now(),
            run_id: self.run_id.clone(),
            agent_identity: self.agent_identity.clone(),
            parent_event_id,
            workspace_path: self.workspace_path.clone(),
        };
        let event = V2AuditEvent { envelope, kind };
        if let Some(sink) = &self.envelope_sink {
            // Disk persistence failures should not crash the run. Emitting
            // the event to the in-memory snapshot is the load-bearing path;
            // JSONL is for review-time inspection.
            let _ = sink.write(&event);
        }
        self.events
            .lock()
            .map_err(|_| WriteError::Poisoned)?
            .push(event);
        Ok(event_id)
    }

    /// Push a parent context so subsequent events nest beneath it.
    pub fn push_parent(&self, event_id: String) -> Result<(), WriteError> {
        let thread_id = std::thread::current().id();
        self.parent_stacks
            .lock()
            .map_err(|_| WriteError::Poisoned)?
            .entry(thread_id)
            .or_default()
            .push(event_id);
        Ok(())
    }

    /// Pop the most recent parent context.
    pub fn pop_parent(&self) -> Result<Option<String>, WriteError> {
        let thread_id = std::thread::current().id();
        let mut stacks = self
            .parent_stacks
            .lock()
            .map_err(|_| WriteError::Poisoned)?;
        let popped = {
            let stack = stacks.entry(thread_id).or_default();
            stack.pop()
        };
        if stacks.get(&thread_id).is_some_and(Vec::is_empty) {
            stacks.remove(&thread_id);
        }
        Ok(popped)
    }

    /// Snapshot the current thread's parent stack so callers can propagate
    /// parentage into spawned worker threads.
    pub(crate) fn parent_stack_snapshot(&self) -> Result<Vec<String>, WriteError> {
        let thread_id = std::thread::current().id();
        Ok(self
            .parent_stacks
            .lock()
            .map_err(|_| WriteError::Poisoned)?
            .get(&thread_id)
            .cloned()
            .unwrap_or_default())
    }

    /// Install a parent stack for the current thread and restore the previous
    /// value when the returned guard is dropped.
    pub(crate) fn install_parent_stack(
        &self,
        stack: Vec<String>,
    ) -> Result<ParentStackGuard<'_>, WriteError> {
        let thread_id = std::thread::current().id();
        let previous = self
            .parent_stacks
            .lock()
            .map_err(|_| WriteError::Poisoned)?
            .insert(thread_id, stack);
        Ok(ParentStackGuard {
            writer: self,
            thread_id,
            previous,
        })
    }

    /// Snapshot of emitted events (for smoke verification).
    pub fn events_snapshot(&self) -> Result<Vec<V2AuditEvent>, WriteError> {
        Ok(self
            .events
            .lock()
            .map_err(|_| WriteError::Poisoned)?
            .clone())
    }

    /// Access to the inner loop-level sink for the loop engine to emit
    /// http.*/tool.call.* events through. Returns a cloned `Arc` so callers
    /// (e.g. `EnforcedAuditSink`) can share ownership without lifetime
    /// gymnastics.
    pub fn inner_sink(&self) -> Arc<dyn AuditSink> {
        Arc::clone(&self.inner)
    }

    /// Proxy: write a blob via the inner sink (sha256-based, per §7.4 / §12 Q11).
    pub fn write_blob(&self, content: &[u8]) -> String {
        self.inner.write_blob(content)
    }

    /// Proxy: emit a loop-level event through the inner sink.
    pub fn emit_loop_event(&self, event: &LoopAuditEvent) {
        self.inner.emit(event);
    }

    fn next_event_id(&self) -> Result<String, WriteError> {
        let mut counter = self
            .event_counter
            .lock()
            .map_err(|_| WriteError::Poisoned)?;
        *counter += 1;
        Ok(format!("v2evt-{}-{:08x}", self.run_id, *counter))
    }

    fn current_parent_event_id(&self) -> Result<Option<String>, WriteError> {
        let thread_id = std::thread::current().id();
        Ok(self
            .parent_stacks
            .lock()
            .map_err(|_| WriteError::Poisoned)?
            .get(&thread_id)
            .and_then(|stack| stack.last().cloned()))
    }
}

impl Drop for ParentStackGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut stacks) = self.writer.parent_stacks.lock() {
            match self.previous.take() {
                Some(previous) if !previous.is_empty() => {
                    stacks.insert(self.thread_id, previous);
                }
                _ => {
                    stacks.remove(&self.thread_id);
                }
            }
        }
    }
}

fn event_type_of(kind: &V2AuditEventKind) -> &'static str {
    match kind {
        V2AuditEventKind::RunStarted { .. } => "run.started",
        V2AuditEventKind::RunFinished { .. } => "run.finished",
        V2AuditEventKind::StepStarted { .. } => "step.started",
        V2AuditEventKind::StepFinished { .. } => "step.finished",
        V2AuditEventKind::StepSkipped { .. } => "step.skipped",
        V2AuditEventKind::StepRetry { .. } => "step.retry",
        V2AuditEventKind::StepDenied { .. } => "step.denied",
        V2AuditEventKind::StepJoin { .. } => "step.join",
        V2AuditEventKind::FanoutDispatched { .. } => "fanout.dispatched",
        V2AuditEventKind::WorkerState { .. } => "worker.state",
        V2AuditEventKind::FaninJoined { .. } => "fanin.joined",
        V2AuditEventKind::LoopIterationStart { .. } => "loop.iteration.start",
        V2AuditEventKind::LoopIterationEnd { .. } => "loop.iteration.end",
        V2AuditEventKind::LoopDidNotConverge { .. } => "loop.did_not_converge",
        V2AuditEventKind::ActivityStarted { .. } => "activity.started",
        V2AuditEventKind::ActivityFinished { .. } => "activity.finished",
        V2AuditEventKind::ToolDenied { .. } => "tool.denied",
    }
}
