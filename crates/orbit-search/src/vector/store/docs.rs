//! Docs-corpus indexing entry points.

use std::collections::BTreeSet;

use orbit_common::types::OrbitError;

use super::{SOURCE_KIND_DOC, VectorStore};
use crate::Embedder;
use crate::vector::UpsertReport;
use crate::vector::doc_fields::{DocEmbeddingSource, doc_embedding_fields};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocReindexReport {
    pub upsert: UpsertReport,
    pub indexed_sources: usize,
    pub stale_sources: Vec<String>,
}

impl VectorStore {
    pub fn index_doc(
        &self,
        doc: &DocEmbeddingSource,
        embedder: &dyn Embedder,
        force: bool,
    ) -> Result<UpsertReport, OrbitError> {
        self.upsert_embeddings(
            SOURCE_KIND_DOC,
            &doc.path,
            &doc_embedding_fields(doc),
            embedder,
            force,
        )
    }

    pub fn reindex_docs(
        &self,
        docs: &[DocEmbeddingSource],
        embedder: &dyn Embedder,
        force: bool,
    ) -> Result<DocReindexReport, OrbitError> {
        let mut upsert = UpsertReport::default();
        let live = docs
            .iter()
            .map(|doc| doc.path.clone())
            .collect::<BTreeSet<_>>();
        for doc in docs {
            let report = self.index_doc(doc, embedder, force)?;
            upsert.embedded_chunks += report.embedded_chunks;
            upsert.skipped_fields += report.skipped_fields;
        }

        let mut stale_sources = Vec::new();
        for source_id in self.source_ids(SOURCE_KIND_DOC)? {
            if !live.contains(&source_id) {
                self.delete_source(SOURCE_KIND_DOC, &source_id)?;
                stale_sources.push(source_id);
            }
        }
        Ok(DocReindexReport {
            upsert,
            indexed_sources: live.len(),
            stale_sources,
        })
    }
}
