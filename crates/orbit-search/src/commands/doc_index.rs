use orbit_common::types::OrbitError;
use serde::Serialize;

use crate::commands::parse_model;
use crate::vector::{DocEmbeddingSource, UpsertReport, VectorStore};
use crate::{Embedder, SubprocessEmbedder};

#[derive(Debug, Clone)]
pub struct DocIndexParams {
    pub model: Option<String>,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocIndexResult {
    pub model_id: String,
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
    run_with_embedder(vector_store, docs, &embedder, params.force)
}

pub(crate) fn run_with_embedder(
    vector_store: &VectorStore,
    docs: &[DocEmbeddingSource],
    embedder: &dyn Embedder,
    force: bool,
) -> Result<DocIndexResult, OrbitError> {
    let report = vector_store.reindex_docs(docs, embedder, force)?;
    Ok(DocIndexResult {
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

    fn doc(path: &str, title: &str, body: &str) -> DocEmbeddingSource {
        DocEmbeddingSource {
            path: path.to_string(),
            title: title.to_string(),
            tags: Vec::new(),
            body: body.to_string(),
        }
    }

    #[test]
    fn doc_index_is_idempotent_by_content_hash() {
        let store = VectorStore::open_in_memory().unwrap();
        let embedder = NoopEmbedder::small();
        let docs = vec![doc("docs/example.md", "Example", "same body")];

        let first = run_with_embedder(&store, &docs, &embedder, false).unwrap();
        let second = run_with_embedder(&store, &docs, &embedder, false).unwrap();

        assert!(first.report.embedded_chunks > 0);
        assert_eq!(second.report.embedded_chunks, 0);
        assert!(second.report.skipped_fields > 0);
    }
}
