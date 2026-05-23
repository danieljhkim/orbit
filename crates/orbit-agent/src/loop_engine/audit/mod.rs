//! Structured audit for the HTTP agent loop.
//!
//! The loop emits a fixed set of structured events — session lifecycle, HTTP
//! request/response, tool-call request/result, iteration boundaries, policy
//! denials — to any [`AuditSink`] implementation. Events carry sha256
//! pointers to redacted payloads stored in a [`BlobStore`]; full bodies live
//! in a separate content-addressed store so events stay small and queryable.
//!
//! Persistent audit storage is owned by the runtime layer. Tests use
//! [`InMemorySink`], callers with no need for persistence use [`NullSink`].

// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::Serialize;

// Re-exports for existing `orbit_agent::...` callers. New code should import
// directly from `orbit_common` — these aliases preserve the public surface
// for the `redaction_smoke` example and downstream crates that already
// import via `orbit_agent::loop_engine::audit`.
pub use orbit_common::utility::blob_store::BlobStore;
pub use orbit_common::utility::redaction::PatternRedactor as RedactionMiddleware;

#[derive(Debug, Clone, Serialize)]
pub struct UsageSnapshot {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event_kind", rename_all = "snake_case")]
pub enum LoopAuditEvent {
    SessionSpawn {
        ts: DateTime<Utc>,
        run_id: String,
        session_id: String,
        provider: String,
        model: String,
        task_id: Option<String>,
        audit_tag: Option<String>,
    },
    SessionClose {
        ts: DateTime<Utc>,
        run_id: String,
        session_id: String,
        reason: String,
    },
    HttpRequest {
        ts: DateTime<Utc>,
        run_id: String,
        session_id: String,
        iteration: u32,
        provider: String,
        model: String,
        endpoint: String,
        body_sha256: String,
    },
    HttpResponse {
        ts: DateTime<Utc>,
        run_id: String,
        session_id: String,
        iteration: u32,
        http_status: u16,
        stop_reason: String,
        usage: UsageSnapshot,
        body_sha256: String,
    },
    ToolCallRequested {
        ts: DateTime<Utc>,
        run_id: String,
        session_id: String,
        iteration: u32,
        tool_name: String,
        tool_use_id: String,
        input_sha256: String,
    },
    ToolCallResult {
        ts: DateTime<Utc>,
        run_id: String,
        session_id: String,
        iteration: u32,
        tool_name: String,
        tool_use_id: String,
        outcome: String,
        output_sha256: String,
        duration_ms: u128,
    },
    IterationBoundary {
        ts: DateTime<Utc>,
        run_id: String,
        session_id: String,
        iteration: u32,
        continues: bool,
    },
    PolicyDenial {
        ts: DateTime<Utc>,
        run_id: String,
        session_id: String,
        iteration: u32,
        tool_name: String,
        reason: String,
    },
}

pub trait AuditSink: Send + Sync {
    fn emit(&self, event: &LoopAuditEvent);
    fn write_blob(&self, content: &[u8]) -> String;
}

pub struct NullSink;

impl AuditSink for NullSink {
    fn emit(&self, _event: &LoopAuditEvent) {}
    fn write_blob(&self, _content: &[u8]) -> String {
        String::new()
    }
}

pub struct InMemorySink {
    events: Mutex<Vec<LoopAuditEvent>>,
    blobs: Mutex<Vec<(String, Vec<u8>)>>,
    blob_store: BlobStore,
}

impl InMemorySink {
    pub fn new(blob_root: impl Into<PathBuf>) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            blobs: Mutex::new(Vec::new()),
            blob_store: BlobStore::new(blob_root),
        }
    }

    pub fn events(&self) -> Vec<LoopAuditEvent> {
        self.events.lock().expect("audit mutex").clone()
    }

    pub fn blob_store(&self) -> &BlobStore {
        &self.blob_store
    }
}

impl AuditSink for InMemorySink {
    fn emit(&self, event: &LoopAuditEvent) {
        self.events.lock().expect("audit mutex").push(event.clone());
    }
    fn write_blob(&self, content: &[u8]) -> String {
        let hash = self
            .blob_store
            .write(content)
            .unwrap_or_else(|err| format!("error:{err}"));
        let stored = self.blob_store.redact_for_storage(content);
        self.blobs
            .lock()
            .expect("blob mutex")
            .push((hash.clone(), stored));
        hash
    }
}

#[cfg(test)]
mod tests;
