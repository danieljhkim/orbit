use orbit_common::types::{OrbitError, Task};
use serde::Serialize;

use crate::commands::parse_model;
use crate::vector::{UpsertReport, VectorStore};
use crate::{Embedder, SubprocessEmbedder};

#[derive(Debug, Clone)]
pub struct SemanticReindexParams {
    pub model: Option<String>,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticReindexResult {
    pub model_id: String,
    pub report: UpsertReport,
}

pub fn run(
    vector_store: &VectorStore,
    tasks: &[Task],
    params: SemanticReindexParams,
) -> Result<SemanticReindexResult, OrbitError> {
    let model = parse_model(params.model.as_deref())?;
    let embedder = SubprocessEmbedder::with_model(model.alias)?;
    let report = vector_store.reindex_tasks(tasks, &embedder, params.force)?;
    Ok(SemanticReindexResult {
        model_id: embedder.model_id().to_string(),
        report,
    })
}
