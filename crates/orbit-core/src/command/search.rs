use std::str::FromStr;

use orbit_common::types::{AdrStatus, LearningStatus, OrbitError, TaskStatus};
use orbit_common::utility::glob::compile_glob_regex;
use orbit_search::{SemanticRelatedParams, SemanticSearchParams};
use orbit_store::LearningSearchParams;
use serde::Serialize;

use crate::{OrbitRuntime, SearchResult};

const DEFAULT_LIMIT: usize = 10;
const DOC_SEARCH_OVERFETCH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GlobalSearchKind {
    Task,
    Doc,
    Learning,
    Adr,
    All,
}

impl GlobalSearchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Doc => "doc",
            Self::Learning => "learning",
            Self::Adr => "adr",
            Self::All => "all",
        }
    }

    fn includes_tasks(self) -> bool {
        matches!(self, Self::Task | Self::All)
    }

    fn includes_docs(self) -> bool {
        matches!(self, Self::Doc | Self::All)
    }

    fn includes_learnings(self) -> bool {
        matches!(self, Self::Learning | Self::All)
    }

    fn includes_adrs(self) -> bool {
        matches!(self, Self::Adr | Self::All)
    }
}

impl Default for GlobalSearchKind {
    fn default() -> Self {
        Self::All
    }
}

impl FromStr for GlobalSearchKind {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "task" => Ok(Self::Task),
            "doc" => Ok(Self::Doc),
            "learning" => Ok(Self::Learning),
            "adr" => Ok(Self::Adr),
            "all" => Ok(Self::All),
            other => Err(format!(
                "invalid search kind `{other}`; expected one of: task, doc, learning, adr, all"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GlobalSearchMode {
    Lexical,
    Hybrid,
    Neighbor,
}

#[derive(Debug, Clone, Default)]
pub struct GlobalSearchParams {
    pub query: Option<String>,
    // ADR-0175: hybrid free-text ranking and task-neighbor lookup are distinct modes.
    pub hybrid: bool,
    pub semantic: Option<String>,
    pub kind: GlobalSearchKind,
    pub limit: usize,
    pub field: Option<String>,
    pub model: Option<String>,
    /// AND-filter by tag. Repeat for multi-tag AND semantics. Applies to
    /// task, doc, learning, ADR (and `all`).
    pub tags: Vec<String>,
    /// Include normally-hidden statuses for the queried kind(s). Mutually
    /// overridden by `status`.
    pub all: bool,
    /// Explicit per-kind status override (set semantics). When non-empty,
    /// takes precedence over the `all` widener.
    pub status: Vec<String>,
    /// Cross-kind applicability filter. Task: selector-mapping against
    /// `context_files`. Learning and ADR: glob-containment against
    /// applicability path globs. Doc: out of scope (returns empty).
    pub path: Option<String>,
}

impl GlobalSearchParams {
    pub fn normalized_limit(&self) -> usize {
        if self.limit == 0 {
            DEFAULT_LIMIT
        } else {
            self.limit
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GlobalSearchResponse {
    pub mode: GlobalSearchMode,
    pub kind: GlobalSearchKind,
    pub results: Vec<GlobalSearchHit>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GlobalSearchHit {
    pub kind: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_by: Option<Vec<String>>,
}

impl OrbitRuntime {
    pub fn global_search(
        &self,
        params: GlobalSearchParams,
    ) -> Result<GlobalSearchResponse, OrbitError> {
        let limit = params.normalized_limit();
        let mut results = Vec::new();
        let mut notes = Vec::new();

        if let Some(semantic_id) = params.semantic {
            if params
                .query
                .as_deref()
                .is_some_and(|query| !query.trim().is_empty())
            {
                return Err(OrbitError::InvalidInput(
                    "`query` and `semantic` are mutually exclusive".to_string(),
                ));
            }
            if !matches!(params.kind, GlobalSearchKind::Task | GlobalSearchKind::All) {
                return Err(OrbitError::InvalidInput(
                    "`semantic` only supports --kind task or --kind all".to_string(),
                ));
            }
            let related = self.semantic_related(SemanticRelatedParams {
                task_id: semantic_id,
                limit,
                model: params.model,
            })?;
            results.extend(related.results.into_iter().map(semantic_hit_to_global));
            return Ok(GlobalSearchResponse {
                mode: GlobalSearchMode::Neighbor,
                kind: params.kind,
                results,
                notes,
            });
        }

        let query_owned = params
            .query
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .map(str::to_string);
        let has_path = params.path.is_some();
        let tag_filter: Vec<String> = params
            .tags
            .iter()
            .map(|tag| tag.trim().to_lowercase())
            .filter(|tag| !tag.is_empty())
            .collect();

        if query_owned.is_none() && !has_path && tag_filter.is_empty() {
            return Err(OrbitError::InvalidInput(
                "search requires a query, --path, or --tag".to_string(),
            ));
        }

        let mode = if params.hybrid {
            GlobalSearchMode::Hybrid
        } else {
            GlobalSearchMode::Lexical
        };

        if params.hybrid && !matches!(params.kind, GlobalSearchKind::Task) {
            notes.push(
                "hybrid vector search currently runs against tasks only; docs, learnings, and ADRs use lexical matching"
                    .to_string(),
            );
        }

        if params.kind.includes_tasks() {
            results.extend(self.task_branch(
                &params,
                query_owned.as_deref(),
                &tag_filter,
                limit,
            )?);
        }

        if params.kind.includes_docs() {
            // Docs are out of scope for `--path`; skip the docs branch entirely
            // when `--path` is set.
            if !has_path {
                results.extend(self.doc_branch(
                    &params,
                    query_owned.as_deref(),
                    &tag_filter,
                    limit,
                )?);
            }
        }

        if params.kind.includes_adrs() {
            results.extend(self.adr_branch(&params, query_owned.as_deref(), &tag_filter, limit)?);
        }

        if params.kind.includes_learnings() {
            results.extend(self.learning_branch(
                &params,
                query_owned.as_deref(),
                &tag_filter,
                limit,
            )?);
        }

        results.truncate(limit);
        Ok(GlobalSearchResponse {
            mode,
            kind: params.kind,
            results,
            notes,
        })
    }

    fn task_branch(
        &self,
        params: &GlobalSearchParams,
        query: Option<&str>,
        tag_filter: &[String],
        limit: usize,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let statuses = resolve_task_statuses(params)?;

        let candidates = if params.hybrid
            && let Some(query) = query
        {
            let search = self.semantic_search(SemanticSearchParams {
                query: query.to_string(),
                limit: limit.saturating_mul(2).max(limit),
                field: params.field.clone(),
                kind: Some("task".to_string()),
                model: params.model.clone(),
            })?;
            // Resolve task records so we can apply the post-filters.
            let mut hits: Vec<(GlobalSearchHit, Option<orbit_common::types::Task>)> = Vec::new();
            for hit in search.results.into_iter() {
                let task = self.get_task(&hit.source_id).ok();
                hits.push((semantic_hit_to_global(hit), task));
            }
            hits
        } else if let Some(query) = query {
            let mut tasks = self.search_tasks_filtered(query, &[])?;
            tasks.truncate(limit.saturating_mul(2).max(limit));
            tasks
                .into_iter()
                .map(|task| (lexical_task_hit(&task), Some(task)))
                .collect()
        } else {
            // No query → enumerate tasks (used by `--path` and `--tag`).
            let tasks = self.list_tasks()?;
            tasks
                .into_iter()
                .map(|task| (lexical_task_hit(&task), Some(task)))
                .collect()
        };

        let path = params.path.as_deref();

        let mut out = Vec::new();
        for (mut hit, task) in candidates {
            let Some(task) = task else { continue };
            if !statuses.contains(&task.status) {
                continue;
            }
            if !tag_filter.is_empty() && !task_has_all_tags(&task, tag_filter) {
                continue;
            }
            if let Some(path) = path
                && !task_selectors_contain_path(&task.context_files, path)
            {
                continue;
            }
            // Override status to keep semantic hits coherent.
            hit.status = Some(task.status.to_string());
            out.push(hit);
        }
        out.truncate(limit);
        Ok(out)
    }

    fn doc_branch(
        &self,
        _params: &GlobalSearchParams,
        query: Option<&str>,
        tag_filter: &[String],
        limit: usize,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let Some(query) = query else {
            if tag_filter.is_empty() {
                // Without a query or tag filter, no doc results — docs are
                // content-indexed, not applicability-indexed.
                return Ok(Vec::new());
            }
            let mut out = Vec::new();
            for record in self.list_docs(None, None)? {
                if !doc_has_all_tags(&record, tag_filter) {
                    continue;
                }
                out.push(GlobalSearchHit {
                    kind: "doc".to_string(),
                    source: "lexical".to_string(),
                    id: None,
                    path: Some(record.path),
                    title: None,
                    summary: Some(record.frontmatter.summary),
                    status: Some(record.frontmatter.doc_type.as_str().to_string()),
                    best_field: None,
                    snippet: None,
                    score: None,
                    matched_by: Some(tag_filter.iter().map(|tag| format!("tag:{tag}")).collect()),
                });
            }
            out.truncate(limit);
            return Ok(out);
        };

        let docs_limit = limit.saturating_mul(DOC_SEARCH_OVERFETCH).max(limit);
        let docs = self.search_docs(query, Some(docs_limit), true)?;
        let mut out = Vec::new();
        for result in docs {
            if let SearchResult::Doc(result) = result {
                if !tag_filter.is_empty() {
                    let record_tags = &result.record.tags;
                    if !tag_filter.iter().all(|tag| {
                        record_tags
                            .iter()
                            .any(|candidate| candidate.eq_ignore_ascii_case(tag))
                    }) {
                        continue;
                    }
                }
                out.push(GlobalSearchHit {
                    kind: "doc".to_string(),
                    source: "lexical".to_string(),
                    id: None,
                    path: Some(result.record.path),
                    title: None,
                    summary: Some(result.record.summary),
                    status: Some(result.record.doc_type),
                    best_field: None,
                    snippet: None,
                    score: Some(result.score as f32),
                    matched_by: Some(result.matched_by),
                });
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    fn adr_branch(
        &self,
        params: &GlobalSearchParams,
        query: Option<&str>,
        tag_filter: &[String],
        limit: usize,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let statuses = resolve_adr_statuses(params)?;
        let path = params.path.as_deref();

        let Some(query) = query else {
            let mut out = Vec::new();
            for adr in self.stores().adrs().list()? {
                if !statuses.contains(&adr.status) {
                    continue;
                }
                if !tag_filter.is_empty() && !adr_has_all_tags(&adr, tag_filter) {
                    continue;
                }
                if let Some(path) = path
                    && !orbit_search::adr_paths_contain_path(&adr.paths, path)?
                {
                    continue;
                }
                out.push(adr_to_global_hit(adr, filter_matched_by(tag_filter, path)));
            }
            out.truncate(limit);
            return Ok(out);
        };

        let docs_limit = limit.saturating_mul(DOC_SEARCH_OVERFETCH).max(limit);
        // Pass `true` so the underlying lexical pass admits superseded ADRs;
        // we apply the status filter ourselves below.
        let docs = self.search_docs(query, Some(docs_limit), true)?;
        let mut out = Vec::new();
        for result in docs {
            if let SearchResult::Adr(result) = result {
                if !statuses.contains(&result.status) {
                    continue;
                }
                if !tag_filter.is_empty() && !adr_result_has_all_tags(&result, tag_filter) {
                    continue;
                }
                if let Some(path) = path
                    && !orbit_search::adr_paths_contain_path(&result.paths, path)?
                {
                    continue;
                }
                out.push(GlobalSearchHit {
                    kind: "adr".to_string(),
                    source: "lexical".to_string(),
                    id: Some(result.id),
                    path: Some(result.path.to_string_lossy().into_owned()),
                    title: Some(result.title),
                    summary: None,
                    status: Some(result.status.to_string()),
                    best_field: None,
                    snippet: None,
                    score: Some(result.score as f32),
                    matched_by: Some(result.matched_by),
                });
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    fn learning_branch(
        &self,
        params: &GlobalSearchParams,
        query: Option<&str>,
        tag_filter: &[String],
        limit: usize,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let statuses = resolve_learning_statuses(params)?;
        let active_only = statuses == vec![LearningStatus::Active];

        // Fast path: when the status set is exactly `[Active]` and we have a
        // query *or* path *or* a single tag, route through the indexed
        // `search_learnings` for speed.
        let single_tag = match tag_filter {
            [tag] => Some(tag.clone()),
            _ => None,
        };
        if active_only && (query.is_some() || params.path.is_some() || single_tag.is_some()) {
            let learnings = self.search_learnings(LearningSearchParams {
                path: params.path.clone(),
                tag: single_tag,
                query: query.map(str::to_string),
                limit: Some(limit.saturating_mul(2).max(limit)),
            })?;
            let mut out = Vec::new();
            for result in learnings {
                // Multi-tag AND filter on top of the index pass.
                if tag_filter.len() > 1 && !learning_has_all_tags(&result.learning, tag_filter) {
                    continue;
                }
                out.push(GlobalSearchHit {
                    kind: "learning".to_string(),
                    source: "lexical".to_string(),
                    id: Some(result.learning.id),
                    path: None,
                    title: None,
                    summary: Some(result.learning.summary),
                    status: Some(result.learning.status.as_str().to_string()),
                    best_field: None,
                    snippet: None,
                    score: None,
                    matched_by: Some(result.matched_by),
                });
            }
            out.truncate(limit);
            return Ok(out);
        }

        // Slow path: enumerate learnings honoring the requested status set,
        // then filter in-memory. Used when `--all`/`--status` widens beyond
        // the active set or when a multi-tag AND is requested.
        let mut out = Vec::new();
        for status in &statuses {
            let learnings = self.list_learnings(Some(*status))?;
            for learning in learnings {
                if let Some(query) = query
                    && !learning
                        .summary
                        .to_lowercase()
                        .contains(&query.to_lowercase())
                {
                    continue;
                }
                if !tag_filter.is_empty() && !learning_has_all_tags(&learning, tag_filter) {
                    continue;
                }
                if let Some(path) = params.path.as_deref()
                    && !learning_scope_contains_path(&learning, path)?
                {
                    continue;
                }
                out.push(GlobalSearchHit {
                    kind: "learning".to_string(),
                    source: "lexical".to_string(),
                    id: Some(learning.id),
                    path: None,
                    title: None,
                    summary: Some(learning.summary),
                    status: Some(learning.status.as_str().to_string()),
                    best_field: None,
                    snippet: None,
                    score: None,
                    matched_by: None,
                });
            }
        }
        out.truncate(limit);
        Ok(out)
    }
}

fn lexical_task_hit(task: &orbit_common::types::Task) -> GlobalSearchHit {
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

fn semantic_hit_to_global(hit: orbit_search::SemanticHit) -> GlobalSearchHit {
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

fn task_has_all_tags(task: &orbit_common::types::Task, tag_filter: &[String]) -> bool {
    tag_filter.iter().all(|needle| {
        task.tags
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(needle))
    })
}

fn learning_has_all_tags(learning: &orbit_common::types::Learning, tag_filter: &[String]) -> bool {
    tag_filter.iter().all(|needle| {
        learning
            .scope
            .tags
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(needle))
    })
}

fn doc_has_all_tags(record: &crate::DocRecord, tag_filter: &[String]) -> bool {
    tag_filter.iter().all(|needle| {
        record
            .frontmatter
            .tags
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(needle))
    })
}

fn adr_has_all_tags(adr: &orbit_common::types::Adr, tag_filter: &[String]) -> bool {
    tag_filter.iter().all(|needle| {
        adr.tags
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(needle))
    })
}

fn adr_result_has_all_tags(result: &orbit_search::AdrSearchResult, tag_filter: &[String]) -> bool {
    tag_filter.iter().all(|needle| {
        result
            .tags
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(needle))
    })
}

fn adr_to_global_hit(
    adr: orbit_common::types::Adr,
    matched_by: Option<Vec<String>>,
) -> GlobalSearchHit {
    let path = std::path::PathBuf::from(".orbit")
        .join("adrs")
        .join(adr.status.cli_name())
        .join(&adr.id)
        .join("body.md");
    GlobalSearchHit {
        kind: "adr".to_string(),
        source: "lexical".to_string(),
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

fn filter_matched_by(tag_filter: &[String], path: Option<&str>) -> Option<Vec<String>> {
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

/// Test whether any of a task's `context_files` selectors apply to `query_path`.
///
/// Selectors take three forms: `file:<path>`, `dir:<path>`, and
/// `symbol:<file>#<name>:<kind>`. A bare path (no prefix) is treated as a
/// file selector. Matching is bidirectional path-containment:
///
/// - exact equality matches.
/// - `query_path` lies within a scope directory.
/// - `scope` lies within a query directory (when the user passes a parent
///   directory, every selector under it matches).
///
/// All three selector forms collapse to a single normalized scope path
/// before the comparison.
pub fn task_selectors_contain_path(selectors: &[String], query_path: &str) -> bool {
    let query = normalize_path_for_match(query_path);
    selectors
        .iter()
        .any(|selector| selector_matches_path(selector, &query))
}

fn selector_matches_path(selector: &str, query: &str) -> bool {
    let scope = if let Some(after) = selector.strip_prefix("file:") {
        after
    } else if let Some(after) = selector.strip_prefix("dir:") {
        after
    } else if let Some(after) = selector.strip_prefix("symbol:") {
        // symbol:<file>#<name>:<kind> — keep only the file portion.
        after.split('#').next().unwrap_or(after)
    } else {
        selector
    };
    let scope = normalize_path_for_match(scope);
    paths_overlap(&scope, query)
}

fn normalize_path_for_match(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("./")
        .trim_start_matches('/')
        .trim_end_matches('/')
        .replace('\\', "/")
}

fn paths_overlap(a: &str, b: &str) -> bool {
    if a == b {
        return !a.is_empty();
    }
    is_within(a, b) || is_within(b, a)
}

fn is_within(inner: &str, outer: &str) -> bool {
    if outer.is_empty() {
        return false;
    }
    if let Some(rest) = inner.strip_prefix(outer) {
        return rest.starts_with('/');
    }
    false
}

fn learning_scope_contains_path(
    learning: &orbit_common::types::Learning,
    query_path: &str,
) -> Result<bool, OrbitError> {
    let normalized = orbit_common::utility::glob::normalize_glob_path(query_path)?;
    for rule in &learning.scope.paths {
        if let Ok(regex) = compile_glob_regex(rule)
            && regex.is_match(&normalized)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn resolve_task_statuses(params: &GlobalSearchParams) -> Result<Vec<TaskStatus>, OrbitError> {
    if !params.status.is_empty() {
        let mut out = Vec::new();
        for raw in &params.status {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            let status = TaskStatus::from_str(trimmed).map_err(OrbitError::InvalidInput)?;
            if !out.contains(&status) {
                out.push(status);
            }
        }
        return Ok(out);
    }
    let mut set = vec![
        TaskStatus::Proposed,
        TaskStatus::Backlog,
        TaskStatus::InProgress,
        TaskStatus::Review,
    ];
    if params.all {
        set.extend([TaskStatus::Done, TaskStatus::Rejected, TaskStatus::Archived]);
    }
    Ok(set)
}

fn resolve_learning_statuses(
    params: &GlobalSearchParams,
) -> Result<Vec<LearningStatus>, OrbitError> {
    if !params.status.is_empty() {
        let mut out = Vec::new();
        for raw in &params.status {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            let status = LearningStatus::from_str(trimmed).map_err(OrbitError::InvalidInput)?;
            if !out.contains(&status) {
                out.push(status);
            }
        }
        return Ok(out);
    }
    let mut set = vec![LearningStatus::Active];
    if params.all {
        set.push(LearningStatus::Superseded);
    }
    Ok(set)
}

fn resolve_adr_statuses(params: &GlobalSearchParams) -> Result<Vec<AdrStatus>, OrbitError> {
    if !params.status.is_empty() {
        let mut out = Vec::new();
        for raw in &params.status {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            let status = AdrStatus::from_str(trimmed).map_err(OrbitError::InvalidInput)?;
            if !out.contains(&status) {
                out.push(status);
            }
        }
        return Ok(out);
    }
    let mut set = vec![AdrStatus::Proposed, AdrStatus::Accepted];
    if params.all {
        set.push(AdrStatus::Superseded);
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use orbit_store::AdrCreateParams;
    use serde_json::json;

    use super::*;

    #[test]
    fn search_modes_serialize_with_public_flag_names() {
        assert_eq!(
            serde_json::to_value(GlobalSearchMode::Lexical).expect("serialize mode"),
            json!("lexical")
        );
        assert_eq!(
            serde_json::to_value(GlobalSearchMode::Hybrid).expect("serialize mode"),
            json!("hybrid")
        );
        assert_eq!(
            serde_json::to_value(GlobalSearchMode::Neighbor).expect("serialize mode"),
            json!("neighbor")
        );
    }

    fn add_tagged_adr(runtime: &OrbitRuntime) -> String {
        runtime
            .stores()
            .adrs()
            .add(AdrCreateParams {
                title: "ADR tag path bridge".to_string(),
                owner: "codex".to_string(),
                related_features: Vec::new(),
                related_tasks: Vec::new(),
                tags: vec!["Perf".to_string(), "orbit-search".to_string()],
                paths: vec!["crates/orbit-search/**".to_string()],
                body: "## Context\n\nTest.\n".to_string(),
            })
            .expect("add adr")
            .id
    }

    #[test]
    fn global_search_adr_tag_filter_matches_case_insensitive() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let adr_id = add_tagged_adr(&runtime);

        let response = runtime
            .global_search(GlobalSearchParams {
                kind: GlobalSearchKind::Adr,
                tags: vec!["perf".to_string()],
                ..Default::default()
            })
            .expect("search by tag");

        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].id.as_deref(), Some(adr_id.as_str()));
        assert_eq!(
            response.results[0].matched_by.as_deref(),
            Some(&["tag:perf".to_string()][..])
        );

        let negative = runtime
            .global_search(GlobalSearchParams {
                kind: GlobalSearchKind::Adr,
                tags: vec!["security".to_string()],
                ..Default::default()
            })
            .expect("search by missing tag");
        assert!(negative.results.is_empty());
    }

    #[test]
    fn global_search_adr_path_filter_matches_glob_containment() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let adr_id = add_tagged_adr(&runtime);

        let response = runtime
            .global_search(GlobalSearchParams {
                kind: GlobalSearchKind::Adr,
                path: Some("crates/orbit-search/src/lib.rs".to_string()),
                ..Default::default()
            })
            .expect("search by path");

        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].id.as_deref(), Some(adr_id.as_str()));
        assert_eq!(
            response.results[0].matched_by.as_deref(),
            Some(&["path:crates/orbit-search/src/lib.rs".to_string()][..])
        );

        let negative = runtime
            .global_search(GlobalSearchParams {
                kind: GlobalSearchKind::Adr,
                path: Some("crates/orbit-core/src/lib.rs".to_string()),
                ..Default::default()
            })
            .expect("search by missing path");
        assert!(negative.results.is_empty());
    }

    #[test]
    fn global_search_all_unions_adr_hits_for_tag_and_path_filters() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let adr_id = add_tagged_adr(&runtime);

        let response = runtime
            .global_search(GlobalSearchParams {
                kind: GlobalSearchKind::All,
                tags: vec!["perf".to_string()],
                path: Some("crates/orbit-search/src/lib.rs".to_string()),
                ..Default::default()
            })
            .expect("search all by tag and path");

        let adr_hit = response
            .results
            .iter()
            .find(|hit| hit.kind == "adr" && hit.id.as_deref() == Some(adr_id.as_str()))
            .expect("adr hit");
        assert_eq!(
            adr_hit.matched_by.as_deref(),
            Some(
                &[
                    "tag:perf".to_string(),
                    "path:crates/orbit-search/src/lib.rs".to_string(),
                ][..]
            )
        );
    }

    #[test]
    fn file_selector_matches_exact_path() {
        let selectors = vec!["file:src/auth/login.rs".to_string()];
        assert!(task_selectors_contain_path(&selectors, "src/auth/login.rs"));
        assert!(!task_selectors_contain_path(
            &selectors,
            "src/auth/logout.rs"
        ));
    }

    #[test]
    fn dir_selector_matches_contained_path() {
        let selectors = vec!["dir:src/auth/".to_string()];
        assert!(task_selectors_contain_path(&selectors, "src/auth/login.rs"));
        assert!(task_selectors_contain_path(
            &selectors,
            "src/auth/handlers/post.rs"
        ));
        assert!(!task_selectors_contain_path(
            &selectors,
            "src/billing/charge.rs"
        ));
    }

    #[test]
    fn symbol_selector_matches_file_component() {
        let selectors = vec!["symbol:src/auth/login.rs#login_handler:function".to_string()];
        assert!(task_selectors_contain_path(&selectors, "src/auth/login.rs"));
        assert!(!task_selectors_contain_path(
            &selectors,
            "src/auth/logout.rs"
        ));
    }

    #[test]
    fn unrelated_dir_selector_does_not_match() {
        let selectors = vec!["dir:crates/orbit-search/".to_string()];
        assert!(!task_selectors_contain_path(
            &selectors,
            "src/auth/login.rs"
        ));
    }

    #[test]
    fn parent_dir_query_matches_descendant_selectors() {
        let selectors = vec![
            "file:src/auth/login.rs".to_string(),
            "dir:src/auth/handlers/".to_string(),
            "symbol:src/auth/logout.rs#logout:function".to_string(),
        ];
        for selector in &selectors {
            assert!(
                task_selectors_contain_path(std::slice::from_ref(selector), "src/auth/"),
                "selector {selector} should match parent dir query"
            );
        }
    }

    #[test]
    fn bare_selector_treated_as_file() {
        let selectors = vec!["src/auth/login.rs".to_string()];
        assert!(task_selectors_contain_path(&selectors, "src/auth/login.rs"));
    }
}
