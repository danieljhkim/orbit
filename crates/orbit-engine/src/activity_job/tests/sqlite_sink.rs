//! ORB-10186: the V2SqliteSink audit writer paths must honor the workspace
//! audit writer/GC guard so the audit collector cannot delete an envelope or a
//! content-addressed blob between its final mark/fingerprint validation and the
//! unlink. Each test holds the same `audit_writer_guard` advisory lock the
//! collector holds during deletion and proves the writer path blocks on it,
//! then completes once the guard is released.

use std::sync::mpsc;
use std::time::Duration;

use chrono::Utc;
use orbit_agent::loop_engine::audit::AuditSink;
use orbit_common::types::activity_job::{
    AUDIT_ENVELOPE_SCHEMA_VERSION, V2AuditEnvelope, V2AuditEvent, V2AuditEventKind,
};
use orbit_common::utility::audit_writer_guard;
use orbit_store::Store;

use crate::activity_job::sqlite_sink::V2SqliteSink;

/// Generous window: an unguarded write completes in well under a millisecond,
/// so a write still pending after this bound is provably blocked on the guard.
const BLOCKED_WINDOW: Duration = Duration::from_millis(500);

fn sink(audit_root: &std::path::Path) -> V2SqliteSink {
    let store = Store::open_in_memory().expect("in-memory store");
    V2SqliteSink::for_audit_root(store, "ws-a", "jrun-test", "codex", None, audit_root)
}

fn run_started_event() -> V2AuditEvent {
    let kind = V2AuditEventKind::RunStarted {
        job_name: "job".to_string(),
        retry_source_run_id: None,
    };
    V2AuditEvent {
        envelope: V2AuditEnvelope {
            schema_version: AUDIT_ENVELOPE_SCHEMA_VERSION,
            event_type: kind.event_type().to_string(),
            event_id: "v2evt-test-00000001".to_string(),
            ts: Utc::now(),
            run_id: "jrun-test".to_string(),
            agent_identity: "codex".to_string(),
            parent_event_id: None,
            workspace_path: None,
        },
        kind,
    }
}

#[test]
fn write_blob_blocks_on_a_held_audit_guard_then_publishes() {
    let temp = tempfile::tempdir().expect("temp");
    let audit_root = temp.path().join("state/audit");
    std::fs::create_dir_all(audit_root.join("blobs")).expect("blob root");
    let sink = sink(&audit_root);

    let guard = audit_writer_guard::acquire(&audit_root).expect("test holds guard");

    let (done_tx, done_rx) = mpsc::channel::<String>();
    std::thread::scope(|threads| {
        threads.spawn(|| {
            // Blocks until the guard is released, then writes the blob.
            let hash = sink.write_blob(b"stdout payload");
            done_tx.send(hash).expect("send hash");
        });

        // While the guard is held the blob write cannot complete.
        assert!(
            matches!(
                done_rx.recv_timeout(BLOCKED_WINDOW),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "write_blob must block while the audit guard is held"
        );

        drop(guard); // release; the blocked write now proceeds

        let hash = done_rx.recv().expect("blob write completes after release");
        assert!(
            !hash.starts_with("error:"),
            "blob write should succeed once the guard is free, got {hash}"
        );
        let blob_path = audit_root.join("blobs").join(&hash[..2]).join(&hash);
        assert!(
            blob_path.exists(),
            "blob must be published after the guard releases"
        );
    });
}

#[test]
fn write_envelope_blocks_on_a_held_audit_guard_then_persists() {
    let temp = tempfile::tempdir().expect("temp");
    let audit_root = temp.path().join("state/audit");
    std::fs::create_dir_all(audit_root.join("blobs")).expect("blob root");
    let sink = sink(&audit_root);
    let event = run_started_event();

    let guard = audit_writer_guard::acquire(&audit_root).expect("test holds guard");

    let (done_tx, done_rx) = mpsc::channel::<Result<(), String>>();
    std::thread::scope(|threads| {
        threads.spawn(|| {
            let result = sink
                .write_envelope(&event)
                .map_err(|error| error.to_string());
            done_tx.send(result).expect("send result");
        });

        assert!(
            matches!(
                done_rx.recv_timeout(BLOCKED_WINDOW),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "write_envelope must block while the audit guard is held"
        );
        assert_eq!(
            sink.persisted_event_count().expect("count"),
            0,
            "no envelope may be persisted while the guard is held"
        );

        drop(guard); // release; the blocked publication now proceeds

        done_rx
            .recv()
            .expect("envelope write completes after release")
            .expect("envelope persists once the guard is free");
    });

    assert_eq!(
        sink.persisted_event_count().expect("count"),
        1,
        "envelope must be persisted after the guard releases"
    );
}
