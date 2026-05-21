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
