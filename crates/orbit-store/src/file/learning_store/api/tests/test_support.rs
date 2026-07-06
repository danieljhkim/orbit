// Shared test fixtures and helpers for the split learning_store/api test suite.
// Keep this file small; individual *_tests.rs pull only what they need.

use crate::backend::LearningCreateParams;
use orbit_common::types::LearningScope;
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

pub(crate) fn store_with_index() -> (TempDir, super::super::store::LearningFileStore) {
    let dir = tempdir().expect("tempdir");
    let index = Store::open_in_memory().expect("open in-memory store");
    let store =
        super::super::store::LearningFileStore::new_with_index(dir.path().to_path_buf(), index);
    (dir, store)
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
