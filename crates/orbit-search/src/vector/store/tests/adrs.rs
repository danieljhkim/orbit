//! Unit tests for `adrs` — sibling layout under store/tests/.

use crate::NoopEmbedder;
use crate::vector::{AdrEmbeddingSource, SOURCE_KIND_ADR, VectorStore};

fn adr(id: &str, title: &str, decision: &str) -> AdrEmbeddingSource {
    AdrEmbeddingSource {
        id: id.to_string(),
        title: title.to_string(),
        body: format!(
            "## Context\nContext for {title}.\n\n## Decision\n{decision}\n\n## Consequences\nConsequences.\n"
        ),
        tags: vec!["search".to_string()],
    }
}

#[test]
fn noop_adr_indexing_populates_adr_rows() {
    let store = VectorStore::open_in_memory().unwrap();
    let embedder = NoopEmbedder::small();

    let report = store
        .index_adr(
            &adr("ADR-0001", "Semantic ADRs", "Index ADR decisions."),
            &embedder,
            false,
        )
        .unwrap();
    let stats = store.stats(&[]).unwrap();

    assert!(report.embedded_chunks >= 4);
    assert_eq!(stats.counts[0].source_kind, "adr");
    assert_eq!(stats.counts[0].model_id, "noop");
}

#[test]
fn reindex_adrs_removes_stale_sources() {
    let store = VectorStore::open_in_memory().unwrap();
    let embedder = NoopEmbedder::small();
    store
        .index_adr(
            &adr("ADR-0001", "Old ADR", "Old decision"),
            &embedder,
            false,
        )
        .unwrap();

    let report = store
        .reindex_adrs(
            &[adr("ADR-0002", "New ADR", "New decision")],
            &embedder,
            false,
        )
        .unwrap();
    let source_ids = store.source_ids(SOURCE_KIND_ADR).unwrap();

    assert_eq!(report.stale_sources, vec!["ADR-0001"]);
    assert_eq!(source_ids.into_iter().collect::<Vec<_>>(), vec!["ADR-0002"]);
}
