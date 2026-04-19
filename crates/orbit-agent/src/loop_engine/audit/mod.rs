//! Structured audit for the HTTP agent loop.
//!
//! The loop emits a fixed set of structured events — session lifecycle, HTTP
//! request/response, tool-call request/result, iteration boundaries, policy
//! denials — to any [`AuditSink`] implementation. Events carry sha256
//! pointers to verbatim payloads stored in a [`BlobStore`]; full bodies live
//! in a separate content-addressed store so events stay small and queryable.
//!
//! The default [`JsonlFileSink`] writes one JSON object per line to
//! `.orbit/audit/loop/{run_id}.jsonl` and fans blob writes to
//! `.orbit/audit/blobs/`. Sinks are pluggable: tests use [`InMemorySink`],
//! callers with no need for persistence use [`NullSink`].

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

// Re-exports for existing `orbit_agent::...` callers. New code should import
// directly from `orbit_common` — these aliases preserve the public surface
// for the `redaction_smoke` example and downstream crates that already
// import via `orbit_agent::loop_engine::audit`.
pub use orbit_common::blob_store::BlobStore;
pub use orbit_common::redaction::PatternRedactor as RedactionMiddleware;

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
        self.blobs
            .lock()
            .expect("blob mutex")
            .push((hash.clone(), content.to_vec()));
        hash
    }
}

pub struct JsonlFileSink {
    run_id: String,
    writer: Mutex<BufWriter<File>>,
    blob_store: Arc<BlobStore>,
    log_path: PathBuf,
}

impl JsonlFileSink {
    pub fn open(audit_root: impl AsRef<Path>, run_id: impl Into<String>) -> std::io::Result<Self> {
        let run_id = run_id.into();
        let root = audit_root.as_ref();
        let loop_dir = root.join("loop");
        fs::create_dir_all(&loop_dir)?;
        let log_path = loop_dir.join(format!("{run_id}.jsonl"));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        let blob_root = root.join("blobs");
        let blob_store = Arc::new(BlobStore::new(blob_root));
        Ok(Self {
            run_id,
            writer: Mutex::new(BufWriter::new(file)),
            blob_store,
            log_path,
        })
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    pub fn blob_store(&self) -> &BlobStore {
        &self.blob_store
    }
}

impl AuditSink for JsonlFileSink {
    fn emit(&self, event: &LoopAuditEvent) {
        let line = match serde_json::to_string(event) {
            Ok(l) => l,
            Err(err) => {
                tracing::warn!("failed to serialize loop audit event: {err}");
                return;
            }
        };
        let mut w = self.writer.lock().expect("audit writer");
        if let Err(err) = writeln!(w, "{line}") {
            tracing::warn!("failed to write loop audit event: {err}");
            return;
        }
        let _ = w.flush();
    }

    fn write_blob(&self, content: &[u8]) -> String {
        self.blob_store
            .write(content)
            .unwrap_or_else(|err| format!("error:{err}"))
    }
}

pub fn json_value_to_vec(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}
