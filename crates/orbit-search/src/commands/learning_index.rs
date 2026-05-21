use orbit_common::types::OrbitError;
use serde::{Deserialize, Serialize};

use crate::commands::parse_model;
use crate::vector::{LearningEmbeddingSource, UpsertReport, VectorStore};
use crate::{Embedder, SubprocessEmbedder};

#[derive(Debug, Clone)]
pub struct LearningIndexParams {
    pub model: Option<String>,
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningIndexResult {
    pub model_id: String,
    pub report: UpsertReport,
    pub indexed_sources: usize,
    pub stale_sources: Vec<String>,
}

pub fn run(
    vector_store: &VectorStore,
    learnings: &[LearningEmbeddingSource],
    params: LearningIndexParams,
) -> Result<LearningIndexResult, OrbitError> {
    let model = parse_model(params.model.as_deref())?;
    let embedder = SubprocessEmbedder::with_model(model.alias)?;
    run_with_embedder(vector_store, learnings, &embedder, params.force)
}

pub(crate) fn run_with_embedder(
    vector_store: &VectorStore,
    learnings: &[LearningEmbeddingSource],
    embedder: &dyn Embedder,
    force: bool,
) -> Result<LearningIndexResult, OrbitError> {
    let report = vector_store.reindex_learnings(learnings, embedder, force)?;
    Ok(LearningIndexResult {
        model_id: embedder.model_id().to_string(),
        report: report.upsert,
        indexed_sources: report.indexed_sources,
        stale_sources: report.stale_sources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoopEmbedder;

    fn learning(id: &str, summary: &str) -> LearningEmbeddingSource {
        LearningEmbeddingSource {
            id: id.to_string(),
            summary: summary.to_string(),
            body: "same body".to_string(),
            tags: vec!["search".to_string()],
        }
    }

    #[test]
    fn learning_index_is_idempotent_by_content_hash() {
        let store = VectorStore::open_in_memory().unwrap();
        let embedder = NoopEmbedder::small();
        let learnings = vec![learning("L-0001", "same summary")];

        let first = run_with_embedder(&store, &learnings, &embedder, false).unwrap();
        let second = run_with_embedder(&store, &learnings, &embedder, false).unwrap();

        assert!(first.report.embedded_chunks > 0);
        assert_eq!(second.report.embedded_chunks, 0);
        assert!(second.report.skipped_fields > 0);
    }
}
