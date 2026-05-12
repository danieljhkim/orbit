use orbit_common::types::{NotFoundKind, OrbitError, Task};
use serde::{Deserialize, Serialize};

use crate::commands::resolve_query_model;
use crate::commands::search::{ScoreBreakdown, SemanticHit, truncate_snippet};
use crate::vector::VectorStore;
use crate::vector::query::{FusedCandidate, cosine_top_k, rollup_to_tasks, snippet_for_hit};
use crate::{Embedder, SubprocessEmbedder};

const DEFAULT_LIMIT: usize = 10;
const RETRIEVER_OVERFETCH: usize = 4;

#[derive(Debug, Clone)]
pub struct SemanticRelatedParams {
    pub task_id: String,
    pub limit: usize,
    pub model: Option<String>,
}

impl SemanticRelatedParams {
    pub fn normalized_limit(&self) -> usize {
        if self.limit == 0 {
            DEFAULT_LIMIT
        } else {
            self.limit
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticRelatedResult {
    pub results: Vec<SemanticHit>,
    pub model_id: String,
}

pub fn run(
    vector_store: &VectorStore,
    tasks: &[Task],
    params: SemanticRelatedParams,
) -> Result<SemanticRelatedResult, OrbitError> {
    let model = resolve_query_model(params.model.as_deref())?;
    let embedder = SubprocessEmbedder::with_model(model.alias)?;
    run_with_embedder(vector_store, tasks, &embedder, params)
}

pub(crate) fn run_with_embedder(
    vector_store: &VectorStore,
    tasks: &[Task],
    embedder: &dyn Embedder,
    params: SemanticRelatedParams,
) -> Result<SemanticRelatedResult, OrbitError> {
    let target = tasks
        .iter()
        .find(|task| task.id == params.task_id)
        .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, params.task_id.clone()))?;
    let query = format!("{}\n\n{}", target.title.trim(), target.description.trim());
    let query = query.trim();
    if query.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "task {} has no embeddable title/description",
            target.id
        )));
    }

    let vectors = embedder.embed(&[query])?;
    let query_vector = vectors.into_iter().next().ok_or_else(|| {
        OrbitError::Execution("embedder returned no vector for semantic related query".to_string())
    })?;
    let limit = params.normalized_limit();
    let retriever_limit = limit.saturating_mul(RETRIEVER_OVERFETCH).max(limit + 1);
    let cosine = cosine_top_k(
        vector_store,
        &query_vector,
        embedder.model_id(),
        retriever_limit,
        Some("task"),
    )?;
    let candidates = cosine
        .into_iter()
        .filter(|hit| hit.source_id != target.id)
        .map(|hit| FusedCandidate {
            source_kind: hit.source_kind,
            source_id: hit.source_id,
            field: hit.field,
            chunk_idx_for_snippet: Some(hit.chunk_idx),
            rowid_for_snippet: None,
            score: hit.score,
            bm25_rank: None,
            cosine_rank: Some(hit.rank),
        })
        .collect::<Vec<_>>();
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
                    rrf: None,
                    bm25_rank: None,
                    cosine_rank: hit.cosine_rank,
                },
            })
        })
        .collect::<Result<Vec<_>, OrbitError>>()?;

    Ok(SemanticRelatedResult {
        results,
        model_id: embedder.model_id().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use orbit_common::types::{Task, TaskPriority, TaskStatus, TaskType};

    use super::*;

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
        if lower.contains("semantic") {
            vec![1.0, 0.0, 0.0]
        } else {
            vec![0.0, 1.0, 0.0]
        }
    }

    fn task(id: &str, title: &str, description: &str) -> Task {
        Task {
            id: id.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            acceptance_criteria: Vec::new(),
            tags: Vec::new(),
            plan: String::new(),
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn related_excludes_self_and_uses_cosine_only() {
        let store = VectorStore::open_in_memory().unwrap();
        let embedder = KeywordEmbedder;
        let tasks = vec![
            task("T1", "semantic search", "design"),
            task("T2", "semantic related", "retrieval"),
            task("T3", "billing", "invoices"),
        ];
        for task in &tasks {
            store.index_task(task, &embedder, false).unwrap();
        }

        let result = run_with_embedder(
            &store,
            &tasks,
            &embedder,
            SemanticRelatedParams {
                task_id: "T1".to_string(),
                limit: 3,
                model: None,
            },
        )
        .unwrap();

        assert!(!result.results.iter().any(|hit| hit.source_id == "T1"));
        assert_eq!(result.results[0].source_id, "T2");
        assert!(result.results[0].score_breakdown.cosine_rank.is_some());
        assert!(result.results[0].score_breakdown.bm25_rank.is_none());
        assert!(result.results[0].score_breakdown.rrf.is_none());
    }
}
