//! Unit tests for `learning` — sibling layout under store/tests/.

use crate::NoopEmbedder;
use crate::vector::{LearningEmbeddingSource, SOURCE_KIND_LEARNING, VectorStore};

fn learning(id: &str, summary: &str) -> LearningEmbeddingSource {
    LearningEmbeddingSource {
        id: id.to_string(),
        summary: summary.to_string(),
        body: "## Why\nConcept retrieval matters.\n\n## How to apply\nSearch by intent.\n"
            .to_string(),
        tags: vec!["search".to_string()],
    }
}

#[test]
fn noop_learning_indexing_populates_learning_rows() {
    let store = VectorStore::open_in_memory().unwrap();
    let embedder = NoopEmbedder::small();

    let report = store
        .index_learning(
            &learning("L-0001", "semantic learning body"),
            &embedder,
            false,
        )
        .unwrap();
    let stats = store.stats(&[]).unwrap();

    assert!(report.embedded_chunks >= 3);
    assert_eq!(stats.counts[0].source_kind, "learning");
    assert_eq!(stats.counts[0].model_id, "noop");
}

#[test]
fn reindex_learnings_removes_stale_sources() {
    let store = VectorStore::open_in_memory().unwrap();
    let embedder = NoopEmbedder::small();
    store
        .index_learning(&learning("L-0001", "old learning"), &embedder, false)
        .unwrap();

    let report = store
        .reindex_learnings(&[learning("L-0002", "new learning")], &embedder, false)
        .unwrap();
    let source_ids = store.source_ids(SOURCE_KIND_LEARNING).unwrap();

    assert_eq!(report.stale_sources, vec!["L-0001"]);
    assert_eq!(source_ids.into_iter().collect::<Vec<_>>(), vec!["L-0002"]);
}
