//! Unit tests for `learning_index` — sibling layout under commands/tests/.

use super::super::learning_index::run_with_embedder;

use crate::NoopEmbedder;
use crate::vector::{LearningEmbeddingSource, VectorStore};

fn learning(id: &str, summary: &str) -> LearningEmbeddingSource {
    LearningEmbeddingSource {
        id: id.to_string(),
        summary: summary.to_string(),
        body: "same body".to_string(),
        tags: vec!["search".to_string()],
    }
}

#[test]
fn learning_index_is_idempotent_by_content_hash() {
    let store = VectorStore::open_in_memory().unwrap();
    let embedder = NoopEmbedder::small();
    let learnings = vec![learning("L-0001", "same summary")];

    let first = run_with_embedder(&store, &learnings, &embedder, false).unwrap();
    let second = run_with_embedder(&store, &learnings, &embedder, false).unwrap();

    assert!(first.report.embedded_chunks > 0);
    assert_eq!(second.report.embedded_chunks, 0);
    assert!(second.report.skipped_fields > 0);
}
