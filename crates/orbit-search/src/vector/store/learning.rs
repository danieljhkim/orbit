//! Project-learning corpus indexing entry points.

use std::collections::BTreeSet;

use orbit_common::types::OrbitError;

use super::{SOURCE_KIND_LEARNING, VectorStore};
use crate::Embedder;
use crate::vector::UpsertReport;
use crate::vector::learning_fields::{LearningEmbeddingSource, learning_embedding_fields};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningReindexReport {
    pub upsert: UpsertReport,
    pub indexed_sources: usize,
    pub stale_sources: Vec<String>,
}

impl VectorStore {
    pub fn index_learning(
        &self,
        learning: &LearningEmbeddingSource,
        embedder: &dyn Embedder,
        force: bool,
    ) -> Result<UpsertReport, OrbitError> {
        self.upsert_embeddings(
            SOURCE_KIND_LEARNING,
            &learning.id,
            &learning_embedding_fields(learning),
            embedder,
            force,
        )
    }

    pub fn reindex_learnings(
        &self,
        learnings: &[LearningEmbeddingSource],
        embedder: &dyn Embedder,
        force: bool,
    ) -> Result<LearningReindexReport, OrbitError> {
        let mut upsert = UpsertReport::default();
        let live = learnings
            .iter()
            .map(|learning| learning.id.clone())
            .collect::<BTreeSet<_>>();
        for learning in learnings {
            let report = self.index_learning(learning, embedder, force)?;
            upsert.embedded_chunks += report.embedded_chunks;
            upsert.skipped_fields += report.skipped_fields;
        }

        let mut stale_sources = Vec::new();
        for source_id in self.source_ids(SOURCE_KIND_LEARNING)? {
            if !live.contains(&source_id) {
                self.delete_source(SOURCE_KIND_LEARNING, &source_id)?;
                stale_sources.push(source_id);
            }
        }
        Ok(LearningReindexReport {
            upsert,
            indexed_sources: live.len(),
            stale_sources,
        })
    }
}
