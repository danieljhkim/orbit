//! Unit tests for `adr_search` — sibling layout under commands/tests/.

use super::super::adr_search::{AdrSemanticSearchParams, run_with_embedder};

use crate::vector::{AdrEmbeddingSource, VectorStore};
use crate::{Embedder, NoopEmbedder};
use orbit_common::types::OrbitError;

struct KeywordEmbedder;

impl Embedder for KeywordEmbedder {
    fn model_id(&self) -> &str {
        "keyword"
    }

    fn dim(&self) -> usize {
        2
    }

    fn max_input_tokens(&self) -> usize {
        512
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, OrbitError> {
        Ok(texts
            .iter()
            .map(|text| {
                if text.to_ascii_lowercase().contains("concept") {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                }
            })
            .collect())
    }

    fn token_count(&self, text: &str) -> Result<usize, OrbitError> {
        Ok(text.split_whitespace().count().max(1))
    }
}

fn adr(id: &str, body: &str) -> AdrEmbeddingSource {
    AdrEmbeddingSource {
        id: id.to_string(),
        title: id.to_string(),
        body: body.to_string(),
        tags: Vec::new(),
    }
}

#[test]
fn adr_semantic_search_filters_to_adr_rows() {
    let store = VectorStore::open_in_memory().unwrap();
    let adr_embedder = KeywordEmbedder;
    store
        .reindex_adrs(
            &[
                adr("ADR-0001", "## Decision\nconcept match\n"),
                adr("ADR-0002", "## Decision\nother body\n"),
            ],
            &adr_embedder,
            false,
        )
        .unwrap();
    store
        .upsert_embeddings(
            "task",
            "ORB-00000",
            &[crate::vector::EmbeddingField::new("title", "concept task")],
            &NoopEmbedder::small(),
            false,
        )
        .unwrap();

    let result = run_with_embedder(
        &store,
        &adr_embedder,
        AdrSemanticSearchParams {
            query: "concept".to_string(),
            limit: 1,
            model: None,
        },
    )
    .unwrap();

    assert_eq!(result.results[0].source_id, "ADR-0001");
}
