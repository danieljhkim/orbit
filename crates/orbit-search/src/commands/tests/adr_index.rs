//! Unit tests for `adr_index` — sibling layout under commands/tests/.

use super::super::adr_index::run_with_embedder;

use crate::NoopEmbedder;
use crate::vector::{AdrEmbeddingSource, VectorStore};

fn adr(id: &str, title: &str, body: &str) -> AdrEmbeddingSource {
    AdrEmbeddingSource {
        id: id.to_string(),
        title: title.to_string(),
        body: body.to_string(),
        tags: Vec::new(),
    }
}

#[test]
fn adr_index_is_idempotent_by_content_hash() {
    let store = VectorStore::open_in_memory().unwrap();
    let embedder = NoopEmbedder::small();
    let adrs = vec![adr(
        "ADR-0001",
        "Stable ADR",
        "## Decision\nKeep the same decision.\n",
    )];

    let first = run_with_embedder(&store, &adrs, &embedder, false).unwrap();
    let second = run_with_embedder(&store, &adrs, &embedder, false).unwrap();

    assert!(first.report.embedded_chunks > 0);
    assert_eq!(second.report.embedded_chunks, 0);
    assert!(second.report.skipped_fields > 0);
}
