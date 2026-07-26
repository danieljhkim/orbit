use orbit_common::types::OrbitError;
use serde::{Deserialize, Serialize};

use crate::commands::parse_model;
use crate::vector::{DocEmbeddingSource, UpsertReport, VectorStore};
use crate::{Embedder, SubprocessEmbedder};

#[derive(Debug, Clone)]
pub struct DocIndexParams {
    pub model: Option<String>,
    pub force: bool,
    /// Checkout whose files supplied this embedding run.
    pub source_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocIndexResult {
    pub model_id: String,
    pub source_root: String,
    pub report: UpsertReport,
    pub indexed_sources: usize,
    pub stale_sources: Vec<String>,
}

pub fn run(
    vector_store: &VectorStore,
    docs: &[DocEmbeddingSource],
    params: DocIndexParams,
) -> Result<DocIndexResult, OrbitError> {
    let model = parse_model(params.model.as_deref())?;
    let embedder = SubprocessEmbedder::with_model(model.alias)?;
    run_with_embedder(
        vector_store,
        docs,
        &embedder,
        params.force,
        params.source_root,
    )
}

pub(crate) fn run_with_embedder(
    vector_store: &VectorStore,
    docs: &[DocEmbeddingSource],
    embedder: &dyn Embedder,
    force: bool,
    source_root: String,
) -> Result<DocIndexResult, OrbitError> {
    let report = vector_store.reindex_docs(docs, embedder, force)?;
    Ok(DocIndexResult {
        model_id: embedder.model_id().to_string(),
        source_root,
        report: report.upsert,
        indexed_sources: report.indexed_sources,
        stale_sources: report.stale_sources,
    })
}
