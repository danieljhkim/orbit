//! ORB-10186: the V2SqliteSink audit writer paths must honor the workspace
//! audit writer/GC guard so the audit collector cannot delete an envelope or a
//! content-addressed blob between its final mark/fingerprint validation and the
//! unlink. Each test holds the same `audit_writer_guard` advisory lock the
//! collector holds during deletion and proves the writer path blocks on it,
//! then completes once the guard is released.

use std::sync::mpsc;
use std::time::Duration;

use chrono::Utc;
use orbit_agent::loop_engine::audit::{AuditSink, LoopAuditEvent};
use orbit_common::types::activity_job::{
    AUDIT_ENVELOPE_SCHEMA_VERSION, V2AuditEnvelope, V2AuditEvent, V2AuditEventKind,
};
use orbit_common::utility::{audit_pending, audit_writer_guard};
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

fn is_pending(audit_root: &std::path::Path, hash: &str) -> bool {
    audit_pending::list(audit_root)
        .expect("list pending markers")
        .iter()
        .any(|marker| marker.hash == hash)
}

/// The whole point of ORB-10186's second round: the *real* production sink
/// sequence is `write_blob` (guard acquired then released) followed later by
/// `write_envelope` (guard re-acquired). The pending-publication marker bridges
/// that gap. Drive the actual sink API — no manual guard, no manual row insert —
/// and prove the marker appears at `write_blob` and is retired only once the
/// referencing envelope is durably persisted, so the collector (which treats a
/// fresh marker as a live reference) can never sweep the blob mid-publication.
#[test]
fn real_write_blob_then_write_envelope_marks_then_clears_pending() {
    let temp = tempfile::tempdir().expect("temp");
    let audit_root = temp.path().join("state/audit");
    std::fs::create_dir_all(audit_root.join("blobs")).expect("blob root");
    let sink = sink(&audit_root);

    // Real production call #1: publish the content-addressed blob.
    let hash = sink.write_blob(b"tool input payload");
    assert!(
        !hash.starts_with("error:"),
        "blob write should succeed: {hash}"
    );
    let blob_path = audit_root.join("blobs").join(&hash[..2]).join(&hash);
    assert!(blob_path.exists(), "blob must be published");
    // Between the two calls the blob is written but unreferenced: only the
    // pending marker keeps GC from sweeping it.
    assert!(
        is_pending(&audit_root, &hash),
        "write_blob must record a pending-publication marker for the new blob"
    );

    // Real production call #2: publish an envelope that references the blob.
    let event = cli_started_event(&hash);
    sink.write_envelope(&event).expect("envelope persists");

    // The reference is now durable, so the marker is retired — the blob is
    // henceforth protected by reachability, not by the pending root.
    assert!(
        !is_pending(&audit_root, &hash),
        "publishing the referencing envelope must clear the pending marker"
    );
    assert!(blob_path.exists(), "the referenced blob must still exist");
    assert_eq!(
        sink.persisted_event_count().expect("count"),
        1,
        "the referencing envelope must be persisted"
    );
}

/// The loop-event path (`emit` → `write_loop_event`) publishes blob references
/// too (e.g. `body_sha256`); it must retire the marker exactly like the envelope
/// path.
#[test]
fn real_write_blob_then_emit_loop_event_clears_pending() {
    let temp = tempfile::tempdir().expect("temp");
    let audit_root = temp.path().join("state/audit");
    std::fs::create_dir_all(audit_root.join("blobs")).expect("blob root");
    let sink = sink(&audit_root);

    let hash = sink.write_blob(b"http request preview");
    assert!(
        is_pending(&audit_root, &hash),
        "marker recorded on write_blob"
    );

    sink.emit(&LoopAuditEvent::HttpRequest {
        ts: Utc::now(),
        run_id: "jrun-test".to_string(),
        session_id: "sess".to_string(),
        iteration: 0,
        provider: "codex".to_string(),
        model: "gpt".to_string(),
        endpoint: String::new(),
        body_sha256: hash.clone(),
    });

    assert!(
        !is_pending(&audit_root, &hash),
        "emitting the loop event that references the blob must clear its marker"
    );
    let blob_path = audit_root.join("blobs").join(&hash[..2]).join(&hash);
    assert!(blob_path.exists(), "the referenced blob survives");
}

fn cli_started_event(stdin_blob_ref: &str) -> V2AuditEvent {
    let kind = V2AuditEventKind::CliInvocationStarted {
        provider: "codex".to_string(),
        argv_redacted: vec!["codex".to_string()],
        stdin_blob_ref: Some(stdin_blob_ref.to_string()),
        model: None,
        cwd: None,
        wall_clock_timeout_ms: 1000,
    };
    V2AuditEvent {
        envelope: V2AuditEnvelope {
            schema_version: AUDIT_ENVELOPE_SCHEMA_VERSION,
            event_type: kind.event_type().to_string(),
            event_id: "v2evt-test-00000002".to_string(),
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
