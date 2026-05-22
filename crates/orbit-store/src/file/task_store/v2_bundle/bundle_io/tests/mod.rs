use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

use chrono::{TimeZone, Utc};
use orbit_common::types::{
    ArtifactManifestFileV2, ArtifactManifestV2, NotFoundKind, OrbitError,
    TASK_ARTIFACT_FILES_DIR_NAME, TASK_ARTIFACT_SCHEMA_VERSION, TASK_ARTIFACTS_DIR_NAME,
    TASK_ENVELOPE_FILE_NAME, TASK_EVENTS_FILE_NAME, TASK_REVIEW_THREADS_DIR_NAME, TaskEventRowV2,
    TaskStatus,
};
use orbit_common::utility::fs::atomic_write_text;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::super::super::tests::test_support::{
    bundle_store, sample_bundle, sample_review_threads,
};
use super::super::*;
use super::read_task_events;

#[test]
fn write_and_read_bundle_round_trips_v2_shape() {
    let temp = TempDir::new().expect("tempdir");
    let store = bundle_store(&temp);
    let mut bundle = sample_bundle("ORB-00000");
    bundle.review_threads = sample_review_threads();

    let created = store.create_bundle(&bundle).expect("create bundle");
    assert_eq!(created.binding.task_id, "ORB-00000");

    let read = store.read_bundle("ORB-00000").expect("read bundle");
    assert_eq!(read.envelope, bundle.envelope);
    assert_eq!(read.description, bundle.description);
    assert_eq!(read.acceptance, bundle.acceptance);
    assert_eq!(read.plan, bundle.plan);
    assert_eq!(read.events, bundle.events);
    assert_eq!(read.comments, bundle.comments);
    assert_eq!(
        read.review_threads
            .iter()
            .map(|thread| thread.metadata.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["RT-0001", "RT-0002"]
    );
    assert_eq!(read.review_threads[0].body, "First thread body");
    assert!(
        created
            .binding
            .canonical_path
            .join(TASK_ENVELOPE_FILE_NAME)
            .is_file()
    );
    assert!(
        created
            .binding
            .canonical_path
            .join(TASK_REVIEW_THREADS_DIR_NAME)
            .is_dir()
    );
    assert!(
        created
            .binding
            .canonical_path
            .join(TASK_ARTIFACTS_DIR_NAME)
            .join(TASK_ARTIFACT_FILES_DIR_NAME)
            .is_dir()
    );
    assert!(
        created
            .binding
            .canonical_path
            .join(TASK_REVIEW_THREADS_DIR_NAME)
            .join("RT-0001.yaml")
            .is_file()
    );
    assert!(
        created
            .binding
            .canonical_path
            .join(TASK_REVIEW_THREADS_DIR_NAME)
            .join("RT-0001.md")
            .is_file()
    );
}

#[test]
fn append_jsonl_repairs_corrupt_tail_only() {
    let temp = TempDir::new().expect("tempdir");
    let store = bundle_store(&temp);
    let bundle = sample_bundle("ORB-00000");
    store.create_bundle(&bundle).expect("create bundle");
    let events_path = store
        .bundle_path("ORB-00000")
        .expect("bundle path")
        .join(TASK_EVENTS_FILE_NAME);
    fs::write(&events_path, "{\"schema_version\":1,\"event_id\":\"EV-0001\",\"at\":\"2026-05-11T12:00:00Z\",\"by\":\"codex:gpt-5.5\",\"type\":\"created\",\"to_status\":\"backlog\"}\n{\"schema_version\"")
        .expect("write corrupt tail");

    store
        .append_event(
            "ORB-00000",
            &TaskEventRowV2 {
                schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                event_id: "EV-0002".to_string(),
                at: Utc.with_ymd_and_hms(2026, 5, 11, 13, 0, 0).unwrap(),
                by: "codex:gpt-5.5".to_string(),
                event_type: "updated".to_string(),
                note: None,
                from_status: None,
                to_status: None,
            },
        )
        .expect("append event");

    let events = read_task_events(&events_path).expect("read events");
    assert_eq!(
        events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["EV-0001", "EV-0002"]
    );
}

#[test]
fn append_jsonl_repairs_trailing_newline_corrupt_tail() {
    let temp = TempDir::new().expect("tempdir");
    let store = bundle_store(&temp);
    store
        .create_bundle(&sample_bundle("ORB-00000"))
        .expect("create bundle");
    let events_path = store
        .bundle_path("ORB-00000")
        .expect("bundle path")
        .join(TASK_EVENTS_FILE_NAME);
    fs::write(&events_path, "{\"schema_version\":1,\"event_id\":\"EV-0001\",\"at\":\"2026-05-11T12:00:00Z\",\"by\":\"codex:gpt-5.5\",\"type\":\"created\",\"to_status\":\"backlog\"}\nnot-json\n")
        .expect("write corrupt tail");

    store
        .append_event(
            "ORB-00000",
            &TaskEventRowV2 {
                schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                event_id: "EV-0002".to_string(),
                at: Utc.with_ymd_and_hms(2026, 5, 11, 13, 0, 0).unwrap(),
                by: "codex:gpt-5.5".to_string(),
                event_type: "updated".to_string(),
                note: None,
                from_status: None,
                to_status: None,
            },
        )
        .expect("append event");

    let events = read_task_events(&events_path).expect("read events");
    assert_eq!(
        events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["EV-0001", "EV-0002"]
    );
}

#[test]
fn append_jsonl_serializes_concurrent_writers() {
    let temp = TempDir::new().expect("tempdir");
    let path = Arc::new(temp.path().join("events.jsonl"));
    let barrier = Arc::new(Barrier::new(8));
    let now = Utc.with_ymd_and_hms(2026, 5, 11, 13, 0, 0).unwrap();
    let handles = (0..8)
        .map(|index| {
            let path = Arc::clone(&path);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                append_jsonl_row(
                    &path,
                    &TaskEventRowV2 {
                        schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                        event_id: format!("EV-{index:04}"),
                        at: now,
                        by: "codex:gpt-5.5".to_string(),
                        event_type: "updated".to_string(),
                        note: None,
                        from_status: None,
                        to_status: None,
                    },
                )
                .expect("append event");
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle.join().expect("join writer");
    }

    let mut ids = read_task_events(&path)
        .expect("read events")
        .into_iter()
        .map(|event| event.event_id)
        .collect::<Vec<_>>();
    ids.sort();
    assert_eq!(
        ids,
        vec![
            "EV-0000", "EV-0001", "EV-0002", "EV-0003", "EV-0004", "EV-0005", "EV-0006", "EV-0007"
        ]
    );
}

#[test]
fn read_jsonl_rejects_corruption_before_tail() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("events.jsonl");
    fs::write(
        &path,
        "{\"schema_version\":1,\"event_id\":\"EV-0001\",\"at\":\"2026-05-11T12:00:00Z\",\"by\":\"codex:gpt-5.5\",\"type\":\"created\",\"to_status\":\"backlog\"}\nnot-json\n{\"schema_version\":1,\"event_id\":\"EV-0002\",\"at\":\"2026-05-11T13:00:00Z\",\"by\":\"codex:gpt-5.5\",\"type\":\"updated\"}\n",
    )
    .expect("write invalid middle");

    assert!(matches!(
        read_task_events(&path),
        Err(OrbitError::Store(message)) if message.contains("before tail")
    ));
}

#[test]
fn read_bundle_rejects_directory_name_that_differs_from_task_id() {
    let temp = TempDir::new().expect("tempdir");
    let store = bundle_store(&temp);
    let created = store
        .create_bundle(&sample_bundle("ORB-00000"))
        .expect("create bundle");
    let renamed = created.binding.canonical_path.with_file_name("ORB-00009");
    fs::rename(&created.binding.canonical_path, &renamed).expect("rename bundle");

    assert!(matches!(
        read_bundle_at(&renamed),
        Err(OrbitError::Store(message)) if message.contains("does not match task id")
    ));
}

#[test]
fn read_bundle_reports_missing_envelope_as_task_not_found() {
    let temp = TempDir::new().expect("tempdir");
    let store = bundle_store(&temp);
    let created = store
        .create_bundle(&sample_bundle("ORB-00000"))
        .expect("create bundle");
    fs::remove_file(created.binding.canonical_path.join(TASK_ENVELOPE_FILE_NAME))
        .expect("remove envelope");

    assert!(matches!(
        store.read_bundle("ORB-00000"),
        Err(OrbitError::NotFound {
            kind: NotFoundKind::Task,
            id: task_id,
        }) if task_id == "ORB-00000"
    ));
}

#[test]
fn read_bundle_rejects_review_thread_metadata_without_body() {
    let temp = TempDir::new().expect("tempdir");
    let store = bundle_store(&temp);
    let mut bundle = sample_bundle("ORB-00000");
    bundle.review_threads = sample_review_threads();
    let created = store.create_bundle(&bundle).expect("create bundle");
    fs::remove_file(
        created
            .binding
            .canonical_path
            .join(TASK_REVIEW_THREADS_DIR_NAME)
            .join("RT-0001.md"),
    )
    .expect("remove thread body");

    assert!(matches!(
        store.read_bundle("ORB-00000"),
        Err(OrbitError::Store(message)) if message.contains("missing task bundle file")
    ));
}

#[test]
fn read_bundle_rejects_manifest_entry_with_missing_artifact_file() {
    let temp = TempDir::new().expect("tempdir");
    let store = bundle_store(&temp);
    store
        .create_bundle(&sample_bundle("ORB-00000"))
        .expect("create bundle");
    let now = Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
    let bundle_dir = store.bundle_path("ORB-00000").expect("bundle path");
    let blob = format!("{TASK_ARTIFACT_FILES_DIR_NAME}/result.txt");
    let blob_path = bundle_dir.join(TASK_ARTIFACTS_DIR_NAME).join(&blob);
    atomic_write_text(&blob_path, "hello").expect("write artifact blob");
    let manifest = ArtifactManifestV2 {
        schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
        files: vec![ArtifactManifestFileV2 {
            path: "result.txt".to_string(),
            blob: blob.clone(),
            sha256: format!("{:x}", Sha256::digest(b"hello")),
            media_type: "text/plain".to_string(),
            size_bytes: 5,
            created_by: "codex:gpt-5.5".to_string(),
            created_at: now,
        }],
    };
    store
        .rewrite_artifact_manifest("ORB-00000", &manifest)
        .expect("write manifest");
    fs::remove_file(blob_path).expect("remove artifact blob");

    assert!(matches!(
        store.read_bundle("ORB-00000"),
        Err(OrbitError::Store(message)) if message.contains("missing file")
    ));
}

#[test]
fn read_bundle_rejects_event_status_newer_than_envelope_status() {
    let temp = TempDir::new().expect("tempdir");
    let store = bundle_store(&temp);
    store
        .create_bundle(&sample_bundle("ORB-00000"))
        .expect("create bundle");
    let mut envelope = sample_bundle("ORB-00000").envelope;
    envelope.status = TaskStatus::InProgress;
    store
        .rewrite_envelope("ORB-00000", &envelope)
        .expect("rewrite mismatched envelope");

    assert!(matches!(
        store.read_bundle("ORB-00000"),
        Err(OrbitError::Store(message)) if message.contains("event log status")
    ));
}
