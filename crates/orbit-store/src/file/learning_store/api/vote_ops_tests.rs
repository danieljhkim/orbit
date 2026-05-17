//! Vote operations tests (upvote, summary, ranking, validation) split per ORB-00116.

use std::sync::Arc;
use std::thread;

use chrono::{TimeZone as _, Utc};
use orbit_common::types::{NotFoundKind, OrbitError};

use super::super::layout::votes_jsonl_path;
use super::super::votes::append_vote_row;
use super::store::LearningFileStore;
use super::test_support::{
    create_params, line_count, set_half_life_env, store_with_index, upvote_params, vote_row,
};
use crate::backend::LearningSearchParams;

#[test]
fn upvote_creates_lazy_votes_file_and_show_summary_reads_it() {
    let (dir, store) = store_with_index();
    let learning = store
        .create_learning(create_params("vote target", vec![], vec![]))
        .expect("create");
    let votes_path = votes_jsonl_path(dir.path(), &learning.id);
    assert!(!votes_path.exists(), "votes file should be lazy");

    let now = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
    let summary = store
        .upvote_learning_at(upvote_params(&learning.id, "claude", Some("ORB-1")), now)
        .expect("upvote");

    assert_eq!(summary.vote_count, 1);
    assert_eq!(summary.last_voted_at, Some(now));
    assert!(votes_path.is_file());
    assert_eq!(line_count(&votes_path), 1);

    let reread = store.learning_vote_summary(&learning.id).expect("summary");
    assert_eq!(reread, summary);
}

#[test]
fn duplicate_upvote_same_key_is_noop_but_cross_task_counts() {
    let (dir, store) = store_with_index();
    let learning = store
        .create_learning(create_params("vote target", vec![], vec![]))
        .expect("create");
    let first = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
    let second = Utc.with_ymd_and_hms(2026, 5, 17, 13, 0, 0).unwrap();
    let third = Utc.with_ymd_and_hms(2026, 5, 17, 14, 0, 0).unwrap();

    store
        .upvote_learning_at(upvote_params(&learning.id, "claude", Some("ORB-1")), first)
        .expect("first");
    let duplicate = store
        .upvote_learning_at(upvote_params(&learning.id, "claude", Some("ORB-1")), second)
        .expect("duplicate");
    assert_eq!(duplicate.vote_count, 1);
    assert_eq!(duplicate.last_voted_at, Some(first));
    assert_eq!(line_count(&votes_jsonl_path(dir.path(), &learning.id)), 1);

    let cross_task = store
        .upvote_learning_at(upvote_params(&learning.id, "claude", Some("ORB-2")), third)
        .expect("cross task");
    assert_eq!(cross_task.vote_count, 2);
    assert_eq!(cross_task.last_voted_at, Some(third));
    assert_eq!(line_count(&votes_jsonl_path(dir.path(), &learning.id)), 2);
}

#[test]
fn upvote_rejects_missing_task_missing_learning_and_superseded_learning() {
    let (dir, store) = store_with_index();
    let learning = store
        .create_learning(create_params("vote target", vec![], vec![]))
        .expect("create");

    let error = store
        .upvote_learning(upvote_params(&learning.id, "claude", None))
        .expect_err("missing task rejected");
    assert!(
        matches!(error, OrbitError::InvalidInput(message) if message.contains("free-floating votes"))
    );
    assert!(!votes_jsonl_path(dir.path(), &learning.id).exists());

    let error = store
        .upvote_learning(upvote_params("L20260517-404", "claude", Some("ORB-1")))
        .expect_err("missing learning rejected");
    assert!(matches!(
        error,
        OrbitError::NotFound {
            kind: NotFoundKind::Learning,
            ..
        }
    ));
    assert!(
        !dir.path()
            .join("L20260517-404")
            .join("votes.jsonl")
            .exists()
    );

    let replacement = store
        .create_learning(create_params("replacement", vec![], vec![]))
        .expect("replacement");
    store
        .supersede_learning(&learning.id, &replacement.id)
        .expect("supersede");
    let error = store
        .upvote_learning(upvote_params(&learning.id, "claude", Some("ORB-2")))
        .expect_err("superseded rejected");
    assert!(matches!(error, OrbitError::InvalidInput(message) if message.contains("superseded")));
}

#[test]
fn per_learning_vote_files_are_isolated() {
    let (_dir, store) = store_with_index();
    let a = store
        .create_learning(create_params("a", vec![], vec![]))
        .expect("a");
    let b = store
        .create_learning(create_params("b", vec![], vec![]))
        .expect("b");
    store
        .upvote_learning(upvote_params(&a.id, "claude", Some("ORB-1")))
        .expect("vote a");

    assert_eq!(
        store
            .learning_vote_summary(&a.id)
            .expect("summary a")
            .vote_count,
        1
    );
    assert_eq!(
        store
            .learning_vote_summary(&b.id)
            .expect("summary b")
            .vote_count,
        0
    );
}

#[test]
fn concurrent_upvotes_append_complete_json_lines() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(LearningFileStore::new(dir.path().to_path_buf()));
    let learning = store
        .create_learning(create_params("concurrent", vec![], vec![]))
        .expect("create");
    let n = 12;
    let mut handles = Vec::new();
    for idx in 0..n {
        let store = Arc::clone(&store);
        let learning_id = learning.id.clone();
        handles.push(thread::spawn(move || {
            store
                .upvote_learning(upvote_params(
                    &learning_id,
                    "claude",
                    Some(&format!("ORB-{idx}")),
                ))
                .expect("upvote");
        }));
    }
    for handle in handles {
        handle.join().expect("thread join");
    }

    let votes_path = votes_jsonl_path(dir.path(), &learning.id);
    let rows = super::super::votes::read_vote_rows(&votes_path).expect("read rows");
    assert_eq!(rows.len(), n);
    assert_eq!(
        store
            .learning_vote_summary(&learning.id)
            .expect("summary")
            .vote_count,
        n
    );
}

#[test]
fn search_ranks_recent_decayed_votes_ahead_of_many_old_votes() {
    let _env = set_half_life_env(None);
    let (dir, store) = store_with_index();
    let now = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
    let recent = store
        .create_learning(create_params("recent", vec!["foo/**"], vec![]))
        .expect("recent");
    let old = store
        .create_learning(create_params("old", vec!["foo/**"], vec![]))
        .expect("old");

    append_vote_row(
        &votes_jsonl_path(dir.path(), &recent.id),
        &vote_row(
            &recent.id,
            "claude",
            "ORB-recent",
            now - chrono::Duration::days(30),
        ),
    )
    .expect("recent vote");
    for idx in 0..3 {
        append_vote_row(
            &votes_jsonl_path(dir.path(), &old.id),
            &vote_row(
                &old.id,
                "claude",
                &format!("ORB-old-{idx}"),
                now - chrono::Duration::days(730 + idx),
            ),
        )
        .expect("old vote");
    }

    let hits = store
        .search_learnings_at(
            LearningSearchParams {
                path: Some("foo/bar.rs".to_string()),
                ..Default::default()
            },
            now,
        )
        .expect("search");

    assert_eq!(hits[0].learning.id, recent.id);
}

#[test]
fn zero_half_life_disables_decay_for_search_ranking() {
    let _env = set_half_life_env(Some("0"));
    let (dir, store) = store_with_index();
    let now = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
    let recent = store
        .create_learning(create_params("recent", vec!["foo/**"], vec![]))
        .expect("recent");
    let old = store
        .create_learning(create_params("old", vec!["foo/**"], vec![]))
        .expect("old");

    append_vote_row(
        &votes_jsonl_path(dir.path(), &recent.id),
        &vote_row(
            &recent.id,
            "claude",
            "ORB-recent",
            now - chrono::Duration::days(30),
        ),
    )
    .expect("recent vote");
    for idx in 0..3 {
        append_vote_row(
            &votes_jsonl_path(dir.path(), &old.id),
            &vote_row(
                &old.id,
                "claude",
                &format!("ORB-old-{idx}"),
                now - chrono::Duration::days(730 + idx),
            ),
        )
        .expect("old vote");
    }

    let hits = store
        .search_learnings_at(
            LearningSearchParams {
                path: Some("foo/bar.rs".to_string()),
                ..Default::default()
            },
            now,
        )
        .expect("search");

    assert_eq!(hits[0].learning.id, old.id);
}

#[test]
fn reindex_validates_votes_and_external_valid_lines_are_visible() {
    let (dir, store) = store_with_index();
    let learning = store
        .create_learning(create_params("target", vec![], vec![]))
        .expect("create");
    let votes_path = votes_jsonl_path(dir.path(), &learning.id);
    let row = vote_row(
        &learning.id,
        "claude",
        "ORB-external",
        Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap(),
    );
    append_vote_row(&votes_path, &row).expect("append external");

    store.reindex_learnings().expect("reindex");
    assert_eq!(
        store
            .learning_vote_summary(&learning.id)
            .expect("summary")
            .vote_count,
        1
    );

    std::fs::write(&votes_path, b"{not-json}\n").expect("write invalid");
    let error = store.reindex_learnings().expect_err("invalid vote line");
    assert!(matches!(error, OrbitError::Store(message) if message.contains("line 1")));
}
