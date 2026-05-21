use std::collections::BTreeMap;

use orbit_common::types::OrbitError;
use serde::{Deserialize, Serialize};

use crate::commands::resolve_query_model;
use crate::vector::VectorStore;
use crate::vector::query::{CosineHit, cosine_top_k, snippet_for_hit};
use crate::vector::store::SOURCE_KIND_LEARNING;
use crate::{Embedder, SubprocessEmbedder};

const DEFAULT_LIMIT: usize = 10;
const RETRIEVER_OVERFETCH: usize = 4;
const SNIPPET_MAX_CHARS: usize = 280;

#[derive(Debug, Clone)]
pub struct LearningSemanticSearchParams {
    pub query: String,
    pub limit: usize,
    pub model: Option<String>,
}

impl LearningSemanticSearchParams {
    pub fn normalized_limit(&self) -> usize {
        if self.limit == 0 {
            DEFAULT_LIMIT
        } else {
            self.limit
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LearningSemanticSearchResult {
    pub results: Vec<LearningSemanticHit>,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LearningSemanticHit {
    pub source_id: String,
    pub best_field: String,
    pub snippet: String,
    pub score: f32,
}

pub fn run(
    vector_store: &VectorStore,
    params: LearningSemanticSearchParams,
) -> Result<LearningSemanticSearchResult, OrbitError> {
    let model = resolve_query_model(params.model.as_deref())?;
    let embedder = SubprocessEmbedder::with_model(model.alias)?;
    run_with_embedder(vector_store, &embedder, params)
}

pub(crate) fn run_with_embedder(
    vector_store: &VectorStore,
    embedder: &dyn Embedder,
    params: LearningSemanticSearchParams,
) -> Result<LearningSemanticSearchResult, OrbitError> {
    let query = params.query.trim();
    if query.is_empty() {
        return Err(OrbitError::InvalidInput(
            "learning semantic search query must not be empty".to_string(),
        ));
    }

    let vectors = embedder.embed(&[query])?;
    let query_vector = vectors.into_iter().next().ok_or_else(|| {
        OrbitError::Execution(
            "embedder returned no vector for learning semantic search".to_string(),
        )
    })?;
    let limit = params.normalized_limit();
    let retriever_limit = limit.saturating_mul(RETRIEVER_OVERFETCH).max(limit);
    let model_id = embedder.model_id().to_string();
    let cosine = cosine_top_k(
        vector_store,
        &query_vector,
        &model_id,
        retriever_limit,
        Some(SOURCE_KIND_LEARNING),
    )?;
    let hits = rollup_learning_hits(vector_store, cosine, limit)?;

    Ok(LearningSemanticSearchResult {
        results: hits,
        model_id,
    })
}

fn rollup_learning_hits(
    vector_store: &VectorStore,
    hits: Vec<CosineHit>,
    limit: usize,
) -> Result<Vec<LearningSemanticHit>, OrbitError> {
    let mut best = BTreeMap::<String, CosineHit>::new();
    for hit in hits {
        best.entry(hit.source_id.clone())
            .and_modify(|current| {
                if crate::vector::query::cosine::compare_cosine_hits(&hit, current).is_lt() {
                    *current = hit.clone();
                }
            })
            .or_insert(hit);
    }

    let mut rolled = Vec::new();
    for hit in best.into_values() {
        let snippet = snippet_for_hit(
            vector_store,
            SOURCE_KIND_LEARNING,
            &hit.source_id,
            &hit.field,
            Some(hit.chunk_idx),
            None,
        )?
        .unwrap_or_default();
        rolled.push(LearningSemanticHit {
            source_id: hit.source_id,
            best_field: hit.field,
            snippet: truncate_snippet(&snippet),
            score: hit.score,
        });
    }
    rolled.sort_by(compare_learning_semantic_hits);
    rolled.truncate(limit);
    Ok(rolled)
}

fn compare_learning_semantic_hits(
    left: &LearningSemanticHit,
    right: &LearningSemanticHit,
) -> std::cmp::Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.source_id.cmp(&right.source_id))
        .then_with(|| left.best_field.cmp(&right.best_field))
}

fn truncate_snippet(snippet: &str) -> String {
    let trimmed = snippet.trim();
    let mut end = 0;
    for (idx, ch) in trimmed.char_indices() {
        if idx > SNIPPET_MAX_CHARS {
            break;
        }
        end = idx + ch.len_utf8();
    }
    if end >= trimmed.len() {
        trimmed.to_string()
    } else {
        format!("{}...", trimmed[..end].trim_end())
    }
}

#[cfg(test)]
mod tests {
    use orbit_common::types::OrbitError;

    use super::*;
    use crate::vector::LearningEmbeddingSource;
    use crate::{Embedder, NoopEmbedder};

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
}
