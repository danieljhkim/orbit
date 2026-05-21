#![allow(missing_docs)]

use super::super::*;

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let seq = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orbit-agent-audit-{name}-{}-{seq}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp test dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn sample_event(run_id: &str) -> LoopAuditEvent {
    LoopAuditEvent::IterationBoundary {
        ts: Utc::now(),
        run_id: run_id.to_string(),
        session_id: "session-1".to_string(),
        iteration: 1,
        continues: false,
    }
}

#[test]
fn jsonl_file_sink_open_is_lazy() {
    let dir = TestDir::new("open-lazy");
    let sink = JsonlFileSink::open(dir.path(), "run-lazy").expect("open sink");

    assert_eq!(
        sink.log_path(),
        dir.path().join("loop/run-lazy.jsonl").as_path()
    );
    assert!(!dir.path().join("loop").exists());
    assert!(!sink.log_path().exists());
}

#[test]
fn jsonl_file_sink_blob_write_does_not_create_loop_file() {
    let dir = TestDir::new("blob-lazy");
    let sink = JsonlFileSink::open(dir.path(), "run-blob").expect("open sink");

    let hash = sink.write_blob(b"stdout payload");

    assert_eq!(hash.len(), 64);
    assert!(!sink.log_path().exists());
    assert!(sink.blob_store().root().exists());
}

#[test]
fn jsonl_file_sink_emit_creates_loop_file() {
    let dir = TestDir::new("emit-lazy");
    let sink = JsonlFileSink::open(dir.path(), "run-event").expect("open sink");

    sink.emit(&sample_event("run-event"));

    let text = std::fs::read_to_string(sink.log_path()).expect("read loop jsonl");
    let line = text.lines().next().expect("event line");
    let event: Value = serde_json::from_str(line).expect("parse event");
    assert_eq!(
        event.get("event_kind").and_then(Value::as_str),
        Some("iteration_boundary")
    );
}
