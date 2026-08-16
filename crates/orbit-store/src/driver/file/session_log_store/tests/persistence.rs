//! Persistence behavior and concurrency coverage.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::thread;
use std::time::Duration;

use tempfile::tempdir;

use crate::fs::lock::acquire_exclusive;

use super::super::{SessionLogAppendParams, SessionLogFilter, SessionLogKind, SessionLogStore};

const LOG_FILE_NAME: &str = "session-log.jsonl";
const LOCK_FILE_NAME: &str = ".session-log.jsonl.lock";

fn append_params(kind: SessionLogKind, body: impl Into<String>) -> SessionLogAppendParams {
    SessionLogAppendParams {
        kind,
        body: body.into(),
        related_task_ids: Vec::new(),
        related_run_ids: Vec::new(),
    }
}

#[test]
fn append_list_resolve_round_trip() {
    let root = tempdir().expect("tempdir");
    let store = SessionLogStore::new(root.path().join(".orbit"));

    let status = store
        .append(append_params(SessionLogKind::Status, "drained nothing"))
        .expect("status");
    assert_eq!(status.id, "SL-0001");
    assert_eq!(status.kind, SessionLogKind::Status);

    let later = store
        .append(SessionLogAppendParams {
            kind: SessionLogKind::CheckLater,
            body: "recheck task after CI".to_string(),
            related_task_ids: vec!["ORB-1".to_string()],
            related_run_ids: Vec::new(),
        })
        .expect("check_later");
    assert_eq!(later.id, "SL-0002");
    assert!(later.resolved_at.is_none());

    let note = store
        .append(append_params(SessionLogKind::Note, "canary owner noted"))
        .expect("note");
    assert_eq!(note.id, "SL-0003");

    let unresolved = store
        .list(SessionLogFilter {
            unresolved_only: true,
            ..SessionLogFilter::default()
        })
        .expect("unresolved");
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].id, "SL-0002");

    let only_notes = store
        .list(SessionLogFilter {
            kind: Some(SessionLogKind::Note),
            ..SessionLogFilter::default()
        })
        .expect("notes");
    assert_eq!(only_notes.len(), 1);
    assert_eq!(only_notes[0].id, "SL-0003");

    let resolved = store.resolve("SL-0002").expect("resolve");
    assert!(resolved.resolved_at.is_some());
    assert!(
        store
            .list(SessionLogFilter {
                unresolved_only: true,
                ..SessionLogFilter::default()
            })
            .expect("after resolve")
            .is_empty()
    );

    let error = store.resolve("SL-0001").expect_err("status cannot resolve");
    assert!(error.to_string().contains("check_later"));
    let error = store.resolve("SL-0002").expect_err("already resolved");
    assert!(error.to_string().contains("already resolved"));
}

#[test]
fn concurrent_appends_allocate_unique_gapless_ids_without_lost_records() {
    const THREADS: usize = 8;
    const APPENDS_PER_THREAD: usize = 20;

    let root = tempdir().expect("tempdir");
    let orbit_dir = root.path().join(".orbit");
    let barrier = Arc::new(Barrier::new(THREADS));
    let mut handles = Vec::new();
    for thread_index in 0..THREADS {
        let orbit_dir = orbit_dir.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            let store = SessionLogStore::new(orbit_dir);
            barrier.wait();
            for append_index in 0..APPENDS_PER_THREAD {
                store
                    .append(append_params(
                        SessionLogKind::Note,
                        format!("thread {thread_index} append {append_index}"),
                    ))
                    .expect("concurrent append");
            }
        }));
    }
    for handle in handles {
        handle.join().expect("append thread");
    }

    let entries = SessionLogStore::new(orbit_dir)
        .list(SessionLogFilter::default())
        .expect("list concurrent appends");
    let expected_count = THREADS * APPENDS_PER_THREAD;
    assert_eq!(entries.len(), expected_count);
    let ids = entries
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), expected_count);
    let expected_ids = (1..=expected_count)
        .map(|id| format!("SL-{id:04}"))
        .collect::<BTreeSet<_>>();
    assert_eq!(ids, expected_ids);
}

#[test]
fn concurrent_append_and_resolve_preserve_every_record_and_resolution() {
    const PAIRS: usize = 24;

    let root = tempdir().expect("tempdir");
    let orbit_dir = root.path().join(".orbit");
    let store = SessionLogStore::new(orbit_dir.clone());
    let check_later_ids = (0..PAIRS)
        .map(|index| {
            store
                .append(append_params(
                    SessionLogKind::CheckLater,
                    format!("resolve target {index}"),
                ))
                .expect("seed check_later")
                .id
        })
        .collect::<Vec<_>>();

    let barrier = Arc::new(Barrier::new(PAIRS * 2));
    let mut handles = Vec::new();
    for (index, id) in check_later_ids.iter().cloned().enumerate() {
        let resolve_orbit_dir = orbit_dir.clone();
        let resolve_barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            let store = SessionLogStore::new(resolve_orbit_dir);
            resolve_barrier.wait();
            store.resolve(&id).expect("concurrent resolve");
        }));

        let append_orbit_dir = orbit_dir.clone();
        let append_barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            let store = SessionLogStore::new(append_orbit_dir);
            append_barrier.wait();
            store
                .append(append_params(
                    SessionLogKind::Note,
                    format!("concurrent note {index}"),
                ))
                .expect("concurrent append");
        }));
    }
    for handle in handles {
        handle.join().expect("mutation thread");
    }

    let entries = store
        .list(SessionLogFilter::default())
        .expect("list after mixed mutations");
    assert_eq!(entries.len(), PAIRS * 2);
    assert_eq!(
        entries
            .iter()
            .map(|entry| &entry.id)
            .collect::<BTreeSet<_>>()
            .len(),
        PAIRS * 2
    );
    for id in check_later_ids {
        let entry = entries
            .iter()
            .find(|entry| entry.id == id)
            .expect("seed record preserved");
        assert!(entry.resolved_at.is_some(), "{id} was not resolved");
    }
}

#[test]
fn recovers_unterminated_final_fragment_without_losing_valid_records() {
    let root = tempdir().expect("tempdir");
    let orbit_dir = root.path().join(".orbit");
    let store = SessionLogStore::new(&orbit_dir);

    store
        .append(append_params(SessionLogKind::Status, "first complete row"))
        .expect("status");
    store
        .append(append_params(
            SessionLogKind::CheckLater,
            "revisit after torn append",
        ))
        .expect("check_later");

    append_raw_bytes(
        &orbit_dir.join(LOG_FILE_NAME),
        br#"{"id":"SL-0003","at":"2026-08-15T18:00:00Z","kind":"note","body":"torn"#,
    );

    let recovered = store
        .list(SessionLogFilter::default())
        .expect("list after torn tail");
    assert_eq!(
        recovered
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["SL-0001", "SL-0002"]
    );

    let raw = fs::read_to_string(orbit_dir.join(LOG_FILE_NAME)).expect("read repaired log");
    assert!(
        raw.ends_with('\n'),
        "recovery must leave a record-boundary newline"
    );
    assert!(
        !raw.contains(r#""id":"SL-0003""#),
        "malformed tail must be truncated, not rewritten in place: {raw}"
    );

    let next = store
        .append(append_params(SessionLogKind::Note, "after recovery"))
        .expect("append after recovery");
    assert_eq!(next.id, "SL-0003");

    let resolved = store.resolve("SL-0002").expect("resolve after recovery");
    assert!(resolved.resolved_at.is_some());

    let listed = store
        .list(SessionLogFilter::default())
        .expect("list after append and resolve");
    assert_eq!(
        listed
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["SL-0001", "SL-0002", "SL-0003"]
    );
    let resolved_row = listed
        .iter()
        .find(|entry| entry.id == "SL-0002")
        .expect("resolved row present");
    assert!(resolved_row.resolved_at.is_some());
    assert_eq!(listed[2].body, "after recovery");
}

#[test]
fn malformed_newline_terminated_rows_still_fail_to_parse() {
    let root = tempdir().expect("tempdir");
    let orbit_dir = root.path().join(".orbit");
    let store = SessionLogStore::new(&orbit_dir);
    store
        .append(append_params(SessionLogKind::Note, "kept prefix"))
        .expect("seed");
    let prefix = fs::read_to_string(orbit_dir.join(LOG_FILE_NAME)).expect("read prefix");

    fs::write(orbit_dir.join(LOG_FILE_NAME), format!("{prefix}not-json\n"))
        .expect("write terminated junk tail");
    let terminated_tail = store
        .list(SessionLogFilter::default())
        .expect_err("terminated junk must not be discarded");
    assert!(
        terminated_tail.to_string().contains("parse"),
        "expected parse error, got {terminated_tail}"
    );
    assert_eq!(
        fs::read_to_string(orbit_dir.join(LOG_FILE_NAME)).expect("reread terminated junk"),
        format!("{prefix}not-json\n"),
        "failed parse must not rewrite the log"
    );

    fs::write(
        orbit_dir.join(LOG_FILE_NAME),
        format!("{prefix}not-json\n{prefix}"),
    )
    .expect("write malformed middle");
    let malformed_middle = store
        .list(SessionLogFilter::default())
        .expect_err("middle junk must not be discarded");
    assert!(
        malformed_middle.to_string().contains("parse"),
        "expected parse error, got {malformed_middle}"
    );
}

#[test]
fn recovery_waits_for_existing_session_log_lock() {
    let root = tempdir().expect("tempdir");
    let orbit_dir = root.path().join(".orbit");
    let store = SessionLogStore::new(&orbit_dir);
    store
        .append(append_params(SessionLogKind::Note, "lock coverage"))
        .expect("seed");
    append_raw_bytes(&orbit_dir.join(LOG_FILE_NAME), b"{\"id\":\"SL-");

    let _guard = acquire_exclusive(&orbit_dir.join(LOCK_FILE_NAME), "session-log test hold")
        .expect("hold lock");
    let started = Arc::new(AtomicBool::new(false));
    let started_for_thread = Arc::clone(&started);
    let (tx, rx) = mpsc::channel();
    let recover_orbit_dir = orbit_dir.clone();
    let handle = thread::spawn(move || {
        started_for_thread.store(true, Ordering::SeqCst);
        let store = SessionLogStore::new(recover_orbit_dir);
        let result = store.list(SessionLogFilter::default());
        tx.send(()).expect("signal completion");
        result
    });

    while !started.load(Ordering::SeqCst) {
        thread::yield_now();
    }
    thread::sleep(Duration::from_millis(80));
    assert!(
        rx.try_recv().is_err(),
        "list/recovery must block on the existing session-log lock"
    );
    drop(_guard);

    rx.recv_timeout(Duration::from_secs(2))
        .expect("list should finish after lock release");
    let recovered = handle
        .join()
        .expect("recover thread")
        .expect("recovered list");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].id, "SL-0001");
}

fn append_raw_bytes(path: &Path, bytes: &[u8]) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open log for raw append");
    file.write_all(bytes).expect("write raw tail");
    file.flush().expect("flush raw tail");
}
