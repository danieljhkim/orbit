// Shared test fixtures and helpers for the split learning_store/api test suite.
// Keep this file small; individual *_tests.rs pull only what they need.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::backend::{LearningCommentAddParams, LearningCreateParams, LearningUpvoteParams};
use chrono::{DateTime, TimeZone, Utc};
use orbit_common::types::{LearningScope, LearningVoteRow};
use tempfile::{TempDir, tempdir};

use crate::Store;

pub(crate) fn create_params(
    summary: &str,
    paths: Vec<&str>,
    tags: Vec<&str>,
) -> LearningCreateParams {
    LearningCreateParams {
        summary: summary.to_string(),
        scope: LearningScope {
            paths: paths.into_iter().map(str::to_string).collect(),
            tags: tags.into_iter().map(str::to_string).collect(),
            ..Default::default()
        },
        body: String::new(),
        evidence: Vec::new(),
        created_by: Some("test".to_string()),
        priority: None,
    }
}

pub(crate) fn store_with_index() -> (TempDir, super::store::LearningFileStore) {
    let dir = tempdir().expect("tempdir");
    let index = Store::open_in_memory().expect("open in-memory store");
    let store = super::store::LearningFileStore::new_with_index(dir.path().to_path_buf(), index);
    (dir, store)
}

pub(crate) fn upvote_params(id: &str, model: &str, task_id: Option<&str>) -> LearningUpvoteParams {
    LearningUpvoteParams {
        learning_id: id.to_string(),
        voter_model: model.to_string(),
        task_id: task_id.map(str::to_string),
    }
}

pub(crate) fn comment_params(id: &str, body: &str) -> LearningCommentAddParams {
    LearningCommentAddParams {
        learning_id: id.to_string(),
        body: body.to_string(),
        author_model: "codex".to_string(),
    }
}

pub(crate) fn vote_row(
    id: &str,
    model: &str,
    task_id: &str,
    voted_at: DateTime<Utc>,
) -> LearningVoteRow {
    LearningVoteRow {
        learning_id: id.to_string(),
        voter_model: model.to_string(),
        voted_at,
        task_id: Some(task_id.to_string()),
    }
}

pub(crate) fn line_count(path: &std::path::Path) -> usize {
    std::fs::read_to_string(path)
        .expect("read votes")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
}

pub(crate) struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    value: Option<String>,
}

pub(crate) fn set_half_life_env(value: Option<&str>) -> EnvGuard {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = std::env::var("ORBIT_LEARNING_VOTE_HALF_LIFE_DAYS").ok();
    unsafe {
        match value {
            Some(value) => std::env::set_var("ORBIT_LEARNING_VOTE_HALF_LIFE_DAYS", value),
            None => std::env::remove_var("ORBIT_LEARNING_VOTE_HALF_LIFE_DAYS"),
        }
    }
    EnvGuard {
        _lock: lock,
        value: previous,
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.value {
                Some(value) => std::env::set_var("ORBIT_LEARNING_VOTE_HALF_LIFE_DAYS", value),
                None => std::env::remove_var("ORBIT_LEARNING_VOTE_HALF_LIFE_DAYS"),
            }
        }
    }
}

pub(crate) fn legacy_learning_yaml(id: &str, status: &str, summary: &str, priority: u8) -> String {
    let second = priority % 10;
    format!(
        "schema_version: 1\n\
         id: {id}\n\
         status: {status}\n\
         scope:\n\
         \x20\x20paths:\n\
         \x20\x20\x20\x20- crates/orbit-store/**\n\
         \x20\x20tags:\n\
         \x20\x20\x20\x20- migration\n\
         summary: {summary}\n\
         body: body for {id}\n\
         evidence:\n\
         \x20\x20- kind: task\n\
         \x20\x20\x20\x20reference: ORB-00096\n\
         created_at: 2026-05-17T00:00:00Z\n\
         updated_at: 2026-05-17T00:00:0{second}Z\n\
         created_by: codex\n\
         priority: {priority}\n"
    )
}
