//! Structured audit for the HTTP agent loop.
//!
//! The loop emits a fixed set of structured events — session lifecycle, HTTP
//! request/response, tool-call request/result, iteration boundaries, policy
//! denials — to any [`AuditSink`] implementation. Events carry sha256
//! pointers to verbatim payloads stored in a [`BlobStore`]; full bodies live
//! in a separate content-addressed store so events stay small and queryable.
//!
//! [`JsonlFileSink`] writes one JSON object per line to
//! `{audit_root}/loop/{run_id}.jsonl` once the first loop event is emitted and
//! fans blob writes to `{audit_root}/blobs/`. Orbit runtime callers pass
//! `.orbit/state/audit` as that root; tests use [`InMemorySink`], callers with
//! no need for persistence use [`NullSink`].

// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
        self.blobs
            .lock()
            .expect("blob mutex")
            .push((hash.clone(), content.to_vec()));
        hash
    }
}

pub struct JsonlFileSink {
    run_id: String,
    writer: Mutex<Option<BufWriter<File>>>,
    blob_store: Arc<BlobStore>,
    log_path: PathBuf,
}

impl JsonlFileSink {
    pub fn open(audit_root: impl AsRef<Path>, run_id: impl Into<String>) -> std::io::Result<Self> {
        let run_id = run_id.into();
        let root = audit_root.as_ref();
        let loop_dir = root.join("loop");
        let log_path = loop_dir.join(format!("{run_id}.jsonl"));
        let blob_root = root.join("blobs");
        let blob_store = Arc::new(BlobStore::new(blob_root));
        Ok(Self {
            run_id,
            writer: Mutex::new(None),
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

    fn ensure_writer<'a>(
        &self,
        writer: &'a mut Option<BufWriter<File>>,
    ) -> io::Result<&'a mut BufWriter<File>> {
        if writer.is_none() {
            if let Some(parent) = self.log_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)?;
            writer.replace(BufWriter::new(file));
        }
        Ok(writer.as_mut().expect("writer initialized"))
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
        let mut writer = self.writer.lock().expect("audit writer");
        let writer = match self.ensure_writer(&mut writer) {
            Ok(writer) => writer,
            Err(err) => {
                tracing::warn!("failed to open loop audit file: {err}");
                return;
            }
        };
        if let Err(err) = writeln!(writer, "{line}") {
            tracing::warn!("failed to write loop audit event: {err}");
            return;
        }
        let _ = writer.flush();
    }

    fn write_blob(&self, content: &[u8]) -> String {
        self.blob_store
            .write(content)
            .unwrap_or_else(|err| format!("error:{err}"))
    }
}

#[cfg(test)]
mod tests;
