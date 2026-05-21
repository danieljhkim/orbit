use orbit_common::types::{OrbitError, TASK_REVIEW_THREADS_DIR_NAME};
use orbit_common::utility::fs::atomic_write_text;
use tempfile::TempDir;

use super::super::tests::test_support::{bundle_store, sample_bundle, sample_review_threads};
use super::REVIEW_THREAD_TOMBSTONES_FILE;

#[test]
fn rewrite_review_threads_validates_before_touching_existing_files() {
    let temp = TempDir::new().expect("tempdir");
    let store = bundle_store(&temp);
    let mut bundle = sample_bundle("ORB-00000");
    bundle.review_threads = sample_review_threads();
    store.create_bundle(&bundle).expect("create bundle");
    let mut invalid = sample_review_threads();
    invalid[0].metadata.path = Some("../escape.rs".to_string());

    assert!(matches!(
        store.rewrite_review_threads("ORB-00000", &invalid),
        Err(OrbitError::InvalidInput(message)) if message.contains("..")
    ));

    let read = store.read_bundle("ORB-00000").expect("read bundle");
    assert_eq!(
        read.review_threads
            .into_iter()
            .map(|thread| thread.metadata.thread_id)
            .collect::<Vec<_>>(),
        vec!["RT-0001", "RT-0002"]
    );
}

#[test]
fn read_review_threads_filters_tombstoned_partial_rewrite_orphans() {
    let temp = TempDir::new().expect("tempdir");
    let store = bundle_store(&temp);
    let mut bundle = sample_bundle("ORB-00000");
    bundle.review_threads = sample_review_threads();
    store.create_bundle(&bundle).expect("create bundle");
    let thread_dir = store
        .bundle_path("ORB-00000")
        .expect("bundle path")
        .join(TASK_REVIEW_THREADS_DIR_NAME);
    atomic_write_text(&thread_dir.join(REVIEW_THREAD_TOMBSTONES_FILE), "RT-0001\n")
        .expect("write tombstones");

    let read = store.read_bundle("ORB-00000").expect("read bundle");
    assert_eq!(
        read.review_threads
            .into_iter()
            .map(|thread| thread.metadata.thread_id)
            .collect::<Vec<_>>(),
        vec!["RT-0002"]
    );
}
