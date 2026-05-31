use super::types::GlobalSearchHit;

pub(super) fn lexical_task_hit(task: &orbit_common::types::Task) -> GlobalSearchHit {
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
        matched_by: None,
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
        matched_by: None,
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
        snippet: None,
        score,
        matched_by: Some(result.matched_by),
    }
}

pub(super) fn adr_result_to_global(
    result: orbit_search::AdrSearchResult,
    source: &str,
) -> GlobalSearchHit {
    GlobalSearchHit {
        kind: "adr".to_string(),
        source: source.to_string(),
        id: Some(result.id),
        path: Some(result.path.to_string_lossy().into_owned()),
        title: Some(result.title),
        summary: None,
        status: Some(result.status.to_string()),
        best_field: None,
        snippet: None,
        score: Some(result.score as f32),
        matched_by: Some(result.matched_by),
    }
}

pub(super) fn adr_to_global_hit(
    adr: orbit_common::types::Adr,
    matched_by: Option<Vec<String>>,
) -> GlobalSearchHit {
    adr_to_global_hit_with_source(adr, "lexical", matched_by)
}

pub(super) fn adr_to_global_hit_with_source(
    adr: orbit_common::types::Adr,
    source: &str,
    matched_by: Option<Vec<String>>,
) -> GlobalSearchHit {
    let path = std::path::PathBuf::from(".orbit")
        .join("adrs")
        .join(adr.status.cli_name())
        .join(&adr.id)
        .join("body.md");
    GlobalSearchHit {
        kind: "adr".to_string(),
        source: source.to_string(),
        id: Some(adr.id),
        path: Some(path.to_string_lossy().into_owned()),
        title: Some(adr.title),
        summary: None,
        status: Some(adr.status.to_string()),
        best_field: None,
        snippet: None,
        score: None,
        matched_by,
    }
}

pub(super) fn filter_matched_by(tag_filter: &[String], path: Option<&str>) -> Option<Vec<String>> {
    let mut matched = Vec::new();
    matched.extend(tag_filter.iter().map(|tag| format!("tag:{tag}")));
    if let Some(path) = path {
        matched.push(format!("path:{path}"));
    }
    if matched.is_empty() {
        None
    } else {
        Some(matched)
    }
}
