//! Unit tests for `doc_index` — sibling layout under commands/tests/.

use super::super::doc_index::run_with_embedder;

use crate::NoopEmbedder;
use crate::vector::{DocEmbeddingSource, VectorStore};

fn doc(path: &str, title: &str, body: &str) -> DocEmbeddingSource {
    DocEmbeddingSource {
        path: path.to_string(),
        title: title.to_string(),
        tags: Vec::new(),
        body: body.to_string(),
    }
}

#[test]
fn doc_index_is_idempotent_by_content_hash() {
    let store = VectorStore::open_in_memory().unwrap();
    let embedder = NoopEmbedder::small();
    let docs = vec![doc("docs/example.md", "Example", "same body")];

    let first = run_with_embedder(&store, &docs, &embedder, false).unwrap();
    let second = run_with_embedder(&store, &docs, &embedder, false).unwrap();

    assert!(first.report.embedded_chunks > 0);
    assert_eq!(second.report.embedded_chunks, 0);
    assert!(second.report.skipped_fields > 0);
}
