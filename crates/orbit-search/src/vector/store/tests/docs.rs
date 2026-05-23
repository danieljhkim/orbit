//! Unit tests for `docs` — sibling layout under store/tests/.

use crate::NoopEmbedder;
use crate::vector::{DocEmbeddingSource, SOURCE_KIND_DOC, VectorStore};

fn doc(path: &str, title: &str, body: &str) -> DocEmbeddingSource {
    DocEmbeddingSource {
        path: path.to_string(),
        title: title.to_string(),
        tags: vec!["docs".to_string()],
        body: body.to_string(),
    }
}

#[test]
fn noop_doc_indexing_populates_doc_rows() {
    let store = VectorStore::open_in_memory().unwrap();
    let embedder = NoopEmbedder::small();

    let report = store
        .index_doc(
            &doc("docs/example.md", "Example", "semantic docs body"),
            &embedder,
            false,
        )
        .unwrap();
    let stats = store.stats(&[]).unwrap();

    assert!(report.embedded_chunks >= 3);
    assert_eq!(stats.counts[0].source_kind, "doc");
    assert_eq!(stats.counts[0].model_id, "noop");
}

#[test]
fn reindex_docs_removes_stale_sources() {
    let store = VectorStore::open_in_memory().unwrap();
    let embedder = NoopEmbedder::small();
    store
        .index_doc(&doc("docs/old.md", "Old", "old body"), &embedder, false)
        .unwrap();

    let report = store
        .reindex_docs(&[doc("docs/new.md", "New", "new body")], &embedder, false)
        .unwrap();
    let source_ids = store.source_ids(SOURCE_KIND_DOC).unwrap();

    assert_eq!(report.stale_sources, vec!["docs/old.md"]);
    assert_eq!(
        source_ids.into_iter().collect::<Vec<_>>(),
        vec!["docs/new.md"]
    );
}
