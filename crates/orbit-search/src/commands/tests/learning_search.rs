//! Unit tests for `learning_search` — sibling layout under commands/tests/.

use super::super::learning_search::{LearningSemanticSearchParams, run_with_embedder};

use crate::vector::{LearningEmbeddingSource, VectorStore};
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

fn learning(id: &str, body: &str) -> LearningEmbeddingSource {
    LearningEmbeddingSource {
        id: id.to_string(),
        summary: id.to_string(),
        body: body.to_string(),
        tags: Vec::new(),
    }
}

#[test]
fn learning_semantic_search_filters_to_learning_rows() {
    let store = VectorStore::open_in_memory().unwrap();
    let learning_embedder = KeywordEmbedder;
    store
        .reindex_learnings(
            &[
                learning("L-0001", "concept match"),
                learning("L-0002", "other body"),
            ],
            &learning_embedder,
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
        &learning_embedder,
        LearningSemanticSearchParams {
            query: "concept".to_string(),
            limit: 1,
            model: None,
        },
    )
    .unwrap();

    assert_eq!(result.results[0].source_id, "L-0001");
}
