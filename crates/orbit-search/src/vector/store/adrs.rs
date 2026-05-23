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
