use orbit_common::types::OrbitError;
use serde::{Deserialize, Serialize};

use crate::commands::parse_model;
use crate::vector::{AdrEmbeddingSource, UpsertReport, VectorStore};
use crate::{Embedder, SubprocessEmbedder};

#[derive(Debug, Clone)]
pub struct AdrIndexParams {
    pub model: Option<String>,
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdrIndexResult {
    pub model_id: String,
    pub report: UpsertReport,
    pub indexed_sources: usize,
    pub stale_sources: Vec<String>,
}

pub fn run(
    vector_store: &VectorStore,
    adrs: &[AdrEmbeddingSource],
    params: AdrIndexParams,
) -> Result<AdrIndexResult, OrbitError> {
    let model = parse_model(params.model.as_deref())?;
    let embedder = SubprocessEmbedder::with_model(model.alias)?;
    run_with_embedder(vector_store, adrs, &embedder, params.force)
}

pub(crate) fn run_with_embedder(
    vector_store: &VectorStore,
    adrs: &[AdrEmbeddingSource],
    embedder: &dyn Embedder,
    force: bool,
) -> Result<AdrIndexResult, OrbitError> {
    let report = vector_store.reindex_adrs(adrs, embedder, force)?;
    Ok(AdrIndexResult {
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
}
