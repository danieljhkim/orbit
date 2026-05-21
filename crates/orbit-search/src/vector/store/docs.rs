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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoopEmbedder;

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
}
