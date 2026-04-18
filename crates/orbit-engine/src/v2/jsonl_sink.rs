//! JSONL sink for §7 v2 audit envelope events.
//!
//! Mirrors `orbit_agent::loop_engine::audit::JsonlFileSink`: one JSON object
//! per line, append-only, flushed per write. Writes to
//! `.orbit/audit/v2_loop/{run_id}.jsonl` so v2 envelope events live alongside
//! — but do not collide with — the loop-level JSONL stream at
//! `.orbit/audit/loop/{run_id}.jsonl`.
//!
//! Used by `V2AuditWriter` callers that want envelope events persisted. In
//! smoke runs this is what lets reviewers open the emitted file and confirm
//! the `run.*`/`step.*`/`activity.*`/`tool.denied` tree.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use orbit_types::v2::V2AuditEvent;

pub struct V2JsonlSink {
    run_id: String,
    writer: Mutex<BufWriter<File>>,
    log_path: PathBuf,
}

impl V2JsonlSink {
    pub fn open(audit_root: impl AsRef<Path>, run_id: impl Into<String>) -> std::io::Result<Self> {
        let run_id = run_id.into();
        let root = audit_root.as_ref();
        let dir = root.join("v2_loop");
        fs::create_dir_all(&dir)?;
        let log_path = dir.join(format!("{run_id}.jsonl"));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        Ok(Self {
            run_id,
            writer: Mutex::new(BufWriter::new(file)),
            log_path,
        })
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    pub fn write(&self, event: &V2AuditEvent) -> std::io::Result<()> {
        let line = serde_json::to_string(event)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut w = self.writer.lock().expect("v2 jsonl writer mutex");
        writeln!(w, "{line}")?;
        w.flush()
    }
}
