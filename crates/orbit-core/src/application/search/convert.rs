use super::types::GlobalSearchHit;

pub(super) fn lexical_task_hit(task: &orbit_types::task::Task) -> GlobalSearchHit {
    GlobalSearchHit {
        kind: "task".to_string(),
        source: "lexical".to_string(),
        id: Some(task.id.clone()),
        path: None,
        title: Some(task.title.clone()),
        summary: Some(task.description.clone()),
        status: Some(task.status.to_string()),
        best_field: None,
        snippet: None,
        score: None,
        score_breakdown: None,
        matched_by: None,
        workspace: None,
    }
}

pub(super) fn semantic_hit_to_global(hit: orbit_search::SemanticHit) -> GlobalSearchHit {
    GlobalSearchHit {
        kind: hit.source_kind,
        source: "semantic".to_string(),
        id: Some(hit.source_id),
        path: None,
        title: None,
        summary: None,
        status: None,
        best_field: Some(hit.best_field),
        snippet: Some(hit.snippet),
        score: Some(hit.score),
        score_breakdown: Some(hit.score_breakdown),
        matched_by: None,
        workspace: None,
    }
}

pub(super) fn doc_result_to_global(
    result: orbit_search::DocSearchResult,
    source: &str,
    score: Option<f32>,
) -> GlobalSearchHit {
    GlobalSearchHit {
        kind: "doc".to_string(),
        source: source.to_string(),
        id: None,
        path: Some(result.record.path),
        title: None,
        summary: Some(result.record.summary),
        status: Some(result.record.doc_type),
        best_field: None,
        snippet: result.snippet,
        score,
        score_breakdown: None,
        matched_by: Some(result.matched_by),
        workspace: None,
    }
}
