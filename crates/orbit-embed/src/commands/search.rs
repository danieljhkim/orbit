use std::thread;

use orbit_common::types::OrbitError;
use serde::{Deserialize, Serialize};

use crate::commands::resolve_query_model;
use crate::vector::VectorStore;
use crate::vector::query::{
    FusedCandidate, bm25_top_k, reciprocal_rank_fusion, rollup_to_tasks, snippet_for_hit,
};
use crate::{Embedder, SubprocessEmbedder};

const DEFAULT_LIMIT: usize = 10;
const RETRIEVER_OVERFETCH: usize = 4;
const SNIPPET_MAX_CHARS: usize = 280;

#[derive(Debug, Clone)]
pub struct SemanticSearchParams {
    pub query: String,
    pub limit: usize,
    pub field: Option<String>,
    pub kind: Option<String>,
    pub model: Option<String>,
}

impl SemanticSearchParams {
    pub fn normalized_limit(&self) -> usize {
        if self.limit == 0 {
            DEFAULT_LIMIT
        } else {
            self.limit
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticSearchResult {
    pub results: Vec<SemanticHit>,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticHit {
    pub source_kind: String,
    pub source_id: String,
    pub best_field: String,
    pub snippet: String,
    pub score: f32,
    pub score_breakdown: ScoreBreakdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoreBreakdown {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rrf: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bm25_rank: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cosine_rank: Option<usize>,
}

pub fn run(
    vector_store: &VectorStore,
    params: SemanticSearchParams,
) -> Result<SemanticSearchResult, OrbitError> {
    let model = resolve_query_model(params.model.as_deref())?;
    let embedder = SubprocessEmbedder::with_model(model.alias)?;
    run_with_embedder(vector_store, &embedder, params)
}

pub(crate) fn run_with_embedder(
    vector_store: &VectorStore,
    embedder: &dyn Embedder,
    params: SemanticSearchParams,
) -> Result<SemanticSearchResult, OrbitError> {
    let query = params.query.trim();
    if query.is_empty() {
        return Err(OrbitError::InvalidInput(
            "semantic search query must not be empty".to_string(),
        ));
    }

    let vectors = embedder.embed(&[query])?;
    let query_vector = vectors.into_iter().next().ok_or_else(|| {
        OrbitError::Execution("embedder returned no vector for semantic search query".to_string())
    })?;
    let limit = params.normalized_limit();
    let retriever_limit = limit.saturating_mul(RETRIEVER_OVERFETCH).max(limit);
    let kind = params.kind.as_deref();
    let model_id = embedder.model_id().to_string();
    let query_for_bm25 = query.to_string();
    let cosine_store = vector_store.clone();
    let bm25_store = vector_store.clone();
    let cosine_model_id = model_id.clone();
    let cosine_kind = kind.map(ToOwned::to_owned);

    let (cosine, bm25) = thread::scope(|scope| {
        let cosine_handle = scope.spawn(|| {
            crate::vector::query::cosine_top_k(
                &cosine_store,
                &query_vector,
                &cosine_model_id,
                retriever_limit,
                cosine_kind.as_deref(),
            )
        });
        let bm25_handle = scope.spawn(|| bm25_top_k(&bm25_store, &query_for_bm25, retriever_limit));
        let cosine = cosine_handle
            .join()
            .map_err(|_| OrbitError::Execution("cosine retriever panicked".to_string()))?;
        let bm25 = bm25_handle
            .join()
            .map_err(|_| OrbitError::Execution("bm25 retriever panicked".to_string()))?;
        Ok::<_, OrbitError>((cosine?, bm25?))
    })?;

    let mut candidates = reciprocal_rank_fusion(&cosine, &bm25);
    apply_candidate_filters(&mut candidates, params.field.as_deref(), kind);
    let task_hits = rollup_to_tasks(candidates, limit);
    let results = task_hits
        .into_iter()
        .map(|hit| {
            let snippet = snippet_for_hit(
                vector_store,
                &hit.source_id,
                &hit.best_field,
                hit.best_chunk_idx,
                hit.best_rowid,
            )?
            .unwrap_or_default();
            Ok(SemanticHit {
                source_kind: hit.source_kind,
                source_id: hit.source_id,
                best_field: hit.best_field,
                snippet: truncate_snippet(&snippet),
                score: hit.score,
                score_breakdown: ScoreBreakdown {
                    rrf: Some(hit.score),
                    bm25_rank: hit.bm25_rank,
                    cosine_rank: hit.cosine_rank,
                },
            })
        })
        .collect::<Result<Vec<_>, OrbitError>>()?;

    Ok(SemanticSearchResult { results, model_id })
}

pub(crate) fn apply_candidate_filters(
    candidates: &mut Vec<FusedCandidate>,
    field: Option<&str>,
    kind: Option<&str>,
) {
    candidates.retain(|candidate| {
        field.is_none_or(|field| candidate.field == field)
            && kind.is_none_or(|kind| candidate.source_kind == kind)
    });
}

pub(crate) fn truncate_snippet(snippet: &str) -> String {
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
    use chrono::Utc;
    use orbit_common::types::{Task, TaskPriority, TaskStatus, TaskType};

    use super::*;
    use crate::vector::VectorStore;

    #[derive(Default)]
    struct KeywordEmbedder;

    impl Embedder for KeywordEmbedder {
        fn model_id(&self) -> &str {
            "keyword"
        }

        fn dim(&self) -> usize {
            3
        }

        fn max_input_tokens(&self) -> usize {
            512
        }

        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, OrbitError> {
            Ok(texts.iter().map(|text| vector_for(text)).collect())
        }

        fn token_count(&self, text: &str) -> Result<usize, OrbitError> {
            Ok(text.split_whitespace().count().max(1))
        }
    }

    fn vector_for(text: &str) -> Vec<f32> {
        let lower = text.to_ascii_lowercase();
        if lower.contains("semantic design") {
            vec![1.0, 0.0, 0.0]
        } else if lower.contains("semantic") {
            vec![0.8, 0.2, 0.0]
        } else {
            vec![0.0, 1.0, 0.0]
        }
    }

    fn task(id: &str, title: &str, description: &str, plan: &str) -> Task {
        Task {
            id: id.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            acceptance_criteria: Vec::new(),
            tags: Vec::new(),
            plan: plan.to_string(),
            execution_summary: String::new(),
            context_files: Vec::new(),
            created_by: None,
            planned_by: None,
            implemented_by: None,
            status: TaskStatus::Backlog,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Chore,
            pr_status: None,
            external_refs: Vec::new(),
            relations: Vec::new(),
            job_run_id: None,
            crew: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn search_runs_both_retrievers_and_rolls_up_fields() {
        let store = VectorStore::open_in_memory().unwrap();
        let embedder = KeywordEmbedder;
        store
            .index_task(
                &task(
                    "T1",
                    "semantic design",
                    "semantic notes",
                    "semantic design appears again in plan",
                ),
                &embedder,
                false,
            )
            .unwrap();
        store
            .index_task(
                &task("T2", "unrelated", "other text", "other plan"),
                &embedder,
                false,
            )
            .unwrap();

        let result = run_with_embedder(
            &store,
            &embedder,
            SemanticSearchParams {
                query: "semantic design".to_string(),
                limit: 10,
                field: None,
                kind: Some("task".to_string()),
                model: None,
            },
        )
        .unwrap();

        let t1_hits = result
            .results
            .iter()
            .filter(|hit| hit.source_id == "T1")
            .collect::<Vec<_>>();
        assert_eq!(t1_hits.len(), 1);
        let breakdown = &t1_hits[0].score_breakdown;
        assert!(breakdown.rrf.is_some());
        assert!(breakdown.bm25_rank.is_some());
        assert!(breakdown.cosine_rank.is_some());
    }
}
