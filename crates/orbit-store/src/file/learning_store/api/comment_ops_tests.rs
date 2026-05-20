//! Comment operations tests (add, list, delete, tombstone, validation, concurrency) split per ORB-00116.

use std::sync::Arc;
use std::thread;

use chrono::{TimeZone, Utc};
use orbit_common::types::{
    LearningCommentEvent, LearningCommentTombstone, NotFoundKind, OrbitError,
};

use super::super::layout::comments_jsonl_path;
use super::super::record::append_jsonl_comment_row;
use super::store::LearningFileStore;
use super::test_support::{comment_params, create_params, line_count, store_with_index};

#[test]
fn learning_comments_round_trip_and_create_file_lazily() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LearningFileStore::new(dir.path().to_path_buf());
    let learning = store
        .create_learning(create_params("target", vec![], vec![]))
        .expect("create");
    let comments_path = comments_jsonl_path(dir.path(), &learning.id);
    assert!(!comments_path.exists());

    let now = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
    let comment = store
        .add_learning_comment_at(comment_params(&learning.id, "  useful note  "), now)
        .expect("comment");

    assert_eq!(comment.id, "C20260517-1");
    assert_eq!(comment.body, "useful note");
    assert!(comments_path.exists());
    let listed = store
        .list_learning_comments(&learning.id, false)
        .expect("list");
    assert_eq!(listed, vec![comment]);
}

#[test]
fn learning_comment_validation_rejects_bad_bodies_and_missing_parent_before_file_creation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LearningFileStore::new(dir.path().to_path_buf());
    let learning = store
        .create_learning(create_params("target", vec![], vec![]))
        .expect("create");

    let too_long = "x".repeat(501);
    for body in ["", "   ", too_long.as_str()] {
        let error = store
            .add_learning_comment(comment_params(&learning.id, body))
            .expect_err("invalid body");
        assert!(matches!(error, OrbitError::InvalidInput(_)));
    }

    let missing = "L-0404";
    let error = store
        .add_learning_comment(comment_params(missing, "valid"))
        .expect_err("missing parent");
    assert!(matches!(
        error,
        OrbitError::NotFound {
            kind: NotFoundKind::Learning,
            ..
        }
    ));
    assert!(!comments_jsonl_path(dir.path(), missing).exists());
}

#[test]
fn learning_comment_rejects_superseded_parent_before_append() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LearningFileStore::new(dir.path().to_path_buf());
    let old = store
        .create_learning(create_params("old", vec![], vec![]))
        .expect("old");
    let new = store
        .create_learning(create_params("new", vec![], vec![]))
        .expect("new");
    store
        .supersede_learning(&old.id, &new.id)
        .expect("supersede");

    let error = store
        .add_learning_comment(comment_params(&old.id, "valid"))
        .expect_err("superseded");
    assert!(
        matches!(error, OrbitError::InvalidInput(message) if message.contains("orbit.learning.supersede"))
    );
    assert!(!comments_jsonl_path(dir.path(), &old.id).exists());
}

#[test]
fn superseding_learning_leaves_comments_on_original_parent_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LearningFileStore::new(dir.path().to_path_buf());
    let old = store
        .create_learning(create_params("old", vec![], vec![]))
        .expect("old");
    let new = store
        .create_learning(create_params("new", vec![], vec![]))
        .expect("new");
    let comment = store
        .add_learning_comment(comment_params(&old.id, "old note"))
        .expect("comment");

    store
        .supersede_learning(&old.id, &new.id)
        .expect("supersede");

    assert!(comments_jsonl_path(dir.path(), &old.id).exists());
    assert_eq!(
        store
            .list_learning_comments(&old.id, false)
            .expect("old comments"),
        vec![comment]
    );
    assert!(
        store
            .list_learning_comments(&new.id, false)
            .expect("new comments")
            .is_empty()
    );
}

#[test]
fn learning_comment_delete_is_tombstone_idempotent_and_include_deleted_restores() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LearningFileStore::new(dir.path().to_path_buf());
    let learning = store
        .create_learning(create_params("target", vec![], vec![]))
        .expect("create");
    let comment = store
        .add_learning_comment_at(
            comment_params(&learning.id, "delete me"),
            Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap(),
        )
        .expect("comment");
    let path = comments_jsonl_path(dir.path(), &learning.id);

    store
        .delete_learning_comment(crate::backend::LearningCommentDeleteParams {
            comment_id: comment.id.clone(),
            deleted_by: "codex".to_string(),
        })
        .expect("delete");
    store
        .delete_learning_comment(crate::backend::LearningCommentDeleteParams {
            comment_id: comment.id.clone(),
            deleted_by: "codex".to_string(),
        })
        .expect("delete again");

    assert!(
        store
            .list_learning_comments(&learning.id, false)
            .expect("list active")
            .is_empty()
    );
    assert_eq!(
        store
            .list_learning_comments(&learning.id, true)
            .expect("list deleted"),
        vec![comment]
    );
    assert_eq!(line_count(&path), 2);
}

#[test]
fn tombstone_before_create_suppresses_comment_on_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LearningFileStore::new(dir.path().to_path_buf());
    let learning = store
        .create_learning(create_params("target", vec![], vec![]))
        .expect("create");
    let path = comments_jsonl_path(dir.path(), &learning.id);
    let ts = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
    append_jsonl_comment_row(
        &path,
        &LearningCommentEvent::Tombstone(LearningCommentTombstone {
            id: "C20260517-1".to_string(),
            learning_id: learning.id.clone(),
            op: "delete".to_string(),
            deleted_at: ts,
            deleted_by: "codex".to_string(),
        }),
    )
    .expect("append tombstone");
    append_jsonl_comment_row(
        &path,
        &LearningCommentEvent::Create(orbit_common::types::LearningComment {
            id: "C20260517-1".to_string(),
            learning_id: learning.id.clone(),
            body: "late create".to_string(),
            author_model: "codex".to_string(),
            created_at: ts,
        }),
    )
    .expect("append create");

    assert!(
        store
            .list_learning_comments(&learning.id, true)
            .expect("list")
            .is_empty()
    );
}

#[test]
fn concurrent_learning_comment_adds_persist_complete_lines() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(LearningFileStore::new(dir.path().to_path_buf()));
    let learning = store
        .create_learning(create_params("target", vec![], vec![]))
        .expect("create");
    let mut handles = Vec::new();
    for idx in 0..16 {
        let store = Arc::clone(&store);
        let learning_id = learning.id.clone();
        handles.push(thread::spawn(move || {
            store
                .add_learning_comment(comment_params(&learning_id, &format!("comment {idx}")))
                .expect("add comment")
        }));
    }
    let comments: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().expect("join"))
        .collect();
    let path = comments_jsonl_path(dir.path(), &learning.id);
    let raw = std::fs::read_to_string(&path).expect("read comments");

    assert_eq!(comments.len(), 16);
    assert_eq!(raw.lines().count(), 16);
    for line in raw.lines() {
        let value: serde_json::Value = serde_json::from_str(line).expect("line json");
        assert!(
            value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .is_some()
        );
    }
    let listed = store
        .list_learning_comments(&learning.id, false)
        .expect("list");
    assert_eq!(listed.len(), 16);
}

#[test]
fn reindex_validates_comments_and_external_valid_lines_are_visible() {
    let (dir, store) = store_with_index();
    let learning = store
        .create_learning(create_params("target", vec![], vec![]))
        .expect("create");
    let path = comments_jsonl_path(dir.path(), &learning.id);
    let ts = Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap();
    append_jsonl_comment_row(
        &path,
        &LearningCommentEvent::Create(orbit_common::types::LearningComment {
            id: "C20260517-1".to_string(),
            learning_id: learning.id.clone(),
            body: "external note".to_string(),
            author_model: "codex".to_string(),
            created_at: ts,
        }),
    )
    .expect("append external");

    store.reindex_learnings().expect("reindex");
    assert_eq!(
        store
            .list_learning_comments(&learning.id, false)
            .expect("list")[0]
            .body,
        "external note"
    );

    std::fs::write(&path, b"{not-json}\n").expect("write invalid");
    let error = store.reindex_learnings().expect_err("invalid comment line");
    assert!(matches!(error, OrbitError::Store(message) if message.contains("line 1")));
}
