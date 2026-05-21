//! ADR-corpus indexing entry points.

use std::collections::BTreeSet;

use orbit_common::types::OrbitError;

use super::{SOURCE_KIND_ADR, VectorStore};
use crate::Embedder;
use crate::vector::UpsertReport;
use crate::vector::adr_fields::{AdrEmbeddingSource, adr_embedding_fields};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdrReindexReport {
    pub upsert: UpsertReport,
    pub indexed_sources: usize,
    pub stale_sources: Vec<String>,
}

impl VectorStore {
    pub fn index_adr(
        &self,
        adr: &AdrEmbeddingSource,
        embedder: &dyn Embedder,
        force: bool,
    ) -> Result<UpsertReport, OrbitError> {
        self.upsert_embeddings(
            SOURCE_KIND_ADR,
            &adr.id,
            &adr_embedding_fields(adr),
            embedder,
            force,
        )
    }

    pub fn reindex_adrs(
        &self,
        adrs: &[AdrEmbeddingSource],
        embedder: &dyn Embedder,
        force: bool,
    ) -> Result<AdrReindexReport, OrbitError> {
        let mut upsert = UpsertReport::default();
        let live = adrs
            .iter()
            .map(|adr| adr.id.clone())
            .collect::<BTreeSet<_>>();
        for adr in adrs {
            let report = self.index_adr(adr, embedder, force)?;
            upsert.embedded_chunks += report.embedded_chunks;
            upsert.skipped_fields += report.skipped_fields;
        }

        let mut stale_sources = Vec::new();
        for source_id in self.source_ids(SOURCE_KIND_ADR)? {
            if !live.contains(&source_id) {
                self.delete_source(SOURCE_KIND_ADR, &source_id)?;
                stale_sources.push(source_id);
            }
        }
        Ok(AdrReindexReport {
            upsert,
            indexed_sources: live.len(),
            stale_sources,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoopEmbedder;

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
}
