//! Persistence behavior and concurrency coverage.

use std::collections::BTreeSet;
use std::sync::{Arc, Barrier};

use tempfile::tempdir;

use super::super::{SessionLogAppendParams, SessionLogFilter, SessionLogKind, SessionLogStore};

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
