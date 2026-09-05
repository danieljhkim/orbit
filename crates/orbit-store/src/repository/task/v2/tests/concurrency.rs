//! ORB-10988 / F2026-07-119: a write to one task must never fail a read or a
//! write of another.
//!
//! The registry binding list is a snapshot and a bundle is a directory, so an
//! unrelated create or delete is observable to a reader as a bundle that is
//! missing or half there. These tests pin both halves of the rule: transient
//! states are skipped, genuine corruption still fails fast.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::driver::file::task_bundle::task_bundle_lock_sentinel_path;

fn create_tasks(store: &TaskV2Store, count: usize) -> Vec<String> {
    (0..count)
        .map(|index| {
            store
                .create_task(create_params(&format!("Task {index}"), TaskStatus::Backlog))
                .expect("create task")
                .id
        })
        .collect()
}

fn document_update(actor: &str, summary: &str) -> TaskDocumentUpdateParams {
    TaskDocumentUpdateParams {
        actor: actor.to_string(),
        execution_summary: Some(summary.to_string()),
        ..Default::default()
    }
}

/// A bundle removed between the registry snapshot and the read — the window
/// `delete_bundle` opens by unregistering and unlinking as two steps — used to
/// fail the whole listing. It must now cost only that one task.
#[test]
fn listing_survives_a_bundle_removed_under_a_live_registry_binding() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let ids = create_tasks(&store, 3);

    let vanished = store
        .bundle_store
        .bundle_path(&ids[1])
        .expect("bundle path");
    std::fs::remove_dir_all(&vanished).expect("remove bundle out of band");

    let listed: Vec<String> = store
        .list_tasks()
        .expect("an unrelated task's removal must not fail the listing")
        .into_iter()
        .map(|task| task.id)
        .collect();
    assert_eq!(listed.len(), 2, "listed: {listed:?}");
    assert!(!listed.contains(&ids[1]), "listed: {listed:?}");
    assert!(listed.contains(&ids[0]) && listed.contains(&ids[2]));

    assert_eq!(
        store
            .bundle_store
            .list_bundles()
            .expect("list bundles")
            .len(),
        2
    );
}

/// An incomplete bundle whose create/delete lock sentinel is present is a
/// writer's work in progress, not damage: skip it and serve every other task.
#[test]
fn listing_skips_an_incomplete_bundle_held_by_the_lock_sentinel() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let ids = create_tasks(&store, 2);

    let in_flight = store
        .bundle_store
        .bundle_path(&ids[0])
        .expect("bundle path");
    let sentinel = task_bundle_lock_sentinel_path(&in_flight).expect("sentinel path");
    std::fs::write(&sentinel, b"").expect("hold the sentinel");
    std::fs::remove_file(in_flight.join("description.md")).expect("truncate publication");

    let listed: Vec<String> = store
        .list_tasks()
        .expect("an in-flight bundle must not fail the listing")
        .into_iter()
        .map(|task| task.id)
        .collect();
    assert_eq!(listed, vec![ids[1].clone()]);
}

/// The tolerance is narrow on purpose: a bundle that is neither held by a
/// writer nor gone is damaged, and damage must still surface loudly rather
/// than quietly shrinking every listing.
#[test]
fn listing_still_fails_fast_on_a_settled_corrupt_bundle() {
    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let ids = create_tasks(&store, 2);

    let corrupt = store
        .bundle_store
        .bundle_path(&ids[0])
        .expect("bundle path");
    std::fs::remove_file(corrupt.join("description.md")).expect("damage bundle");

    let err = store
        .list_tasks()
        .expect_err("a settled, damaged bundle must not be silently skipped");
    assert!(
        matches!(err, OrbitError::TaskBundleCorrupt { ref task_id, .. } if *task_id == ids[0]),
        "expected corruption for {}, got {err}",
        ids[0]
    );
}

/// The reported failure shape: parallel writes to *distinct* tasks racing the
/// index validation and rebuild that every listing performs. Serially these
/// same calls always succeeded; concurrently they transiently failed.
#[test]
fn parallel_updates_to_distinct_tasks_never_fail_a_concurrent_listing() {
    const TASKS: usize = 8;
    const ROUNDS: usize = 12;

    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let ids = create_tasks(&store, TASKS);
    let listings = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        for (index, id) in ids.iter().enumerate() {
            let store = &store;
            scope.spawn(move || {
                for round in 0..ROUNDS {
                    store
                        .update_task_document(
                            id,
                            &document_update("codex:gpt-5.5", &format!("writer {index} @{round}")),
                        )
                        .unwrap_or_else(|err| {
                            panic!("update of {id} failed under concurrency: {err}")
                        });
                }
            });
        }
        for _ in 0..4 {
            let store = &store;
            let listings = &listings;
            scope.spawn(move || {
                for _ in 0..(ROUNDS * TASKS) {
                    let tasks = store
                        .list_tasks()
                        .unwrap_or_else(|err| panic!("listing failed under concurrency: {err}"));
                    assert_eq!(tasks.len(), TASKS, "no task may drop out of a listing");
                    let page = store
                        .query_task_rows(&Default::default(), 3, None)
                        .unwrap_or_else(|err| {
                            panic!("bounded listing failed under concurrency: {err}")
                        });
                    assert_eq!(page.items.len(), 3);
                    listings.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });

    assert!(listings.load(Ordering::Relaxed) > 0);
    for (index, id) in ids.iter().enumerate() {
        let task = store.get_task(id).expect("get task").expect("task exists");
        assert_eq!(
            task.execution_summary,
            format!("writer {index} @{}", ROUNDS - 1),
            "every writer's last write must be durable"
        );
    }
}

/// Same-task concurrency: the per-task lock must serialize whole updates, so
/// every appended comment survives instead of racing writers overwriting each
/// other's view of the comment sequence.
#[test]
fn parallel_updates_to_one_task_keep_every_appended_comment() {
    const WRITERS: usize = 6;
    const COMMENTS_PER_WRITER: usize = 5;

    let temp = TempDir::new().expect("tempdir");
    let store = store(&temp);
    let id = create_tasks(&store, 1).remove(0);
    let created_comments = store
        .get_task_comments(&id)
        .expect("get comments")
        .expect("task exists")
        .len();

    std::thread::scope(|scope| {
        for writer in 0..WRITERS {
            let store = &store;
            let id = &id;
            scope.spawn(move || {
                for round in 0..COMMENTS_PER_WRITER {
                    store
                        .update_task_history(
                            id,
                            &TaskHistoryUpdateParams {
                                actor: "codex:gpt-5.5".to_string(),
                                append_comments: vec![TaskComment {
                                    at: Utc::now(),
                                    by: format!("writer-{writer}"),
                                    message: format!("writer {writer} comment {round}"),
                                }],
                                ..Default::default()
                            },
                        )
                        .unwrap_or_else(|err| panic!("history update failed: {err}"));
                }
            });
        }
    });

    let comments = store
        .get_task_comments(&id)
        .expect("get comments")
        .expect("task exists");
    assert_eq!(
        comments.len(),
        created_comments + WRITERS * COMMENTS_PER_WRITER,
        "no concurrent comment may be lost"
    );
}
