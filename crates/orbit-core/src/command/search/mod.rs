use std::collections::{BTreeMap, VecDeque};

use orbit_common::types::{LearningStatus, OrbitError};
use orbit_search::{
    AdrSemanticHit, AdrSemanticSearchParams, DocSemanticHit, DocSemanticSearchParams,
    LearningSemanticHit, LearningSemanticSearchParams, SemanticRelatedParams, SemanticSearchParams,
};
use orbit_store::LearningSearchParams;

use crate::{OrbitRuntime, SearchResult};

mod convert;
mod filters;
mod hybrid;
mod path_match;
mod types;

#[cfg(test)]
mod tests;

pub use path_match::task_selectors_contain_path;
pub use types::{
    GlobalSearchHit, GlobalSearchKind, GlobalSearchMode, GlobalSearchParams, GlobalSearchResponse,
};

use self::convert::{
    adr_result_to_global, adr_to_global_hit, adr_to_global_hit_with_source, doc_result_to_global,
    filter_matched_by, lexical_task_hit, semantic_hit_to_global,
};
use self::filters::{
    SearchStatusFilters, adr_has_all_tags, adr_result_has_all_tags, doc_has_all_tags,
    learning_has_all_tags, resolve_adr_statuses, resolve_learning_statuses, resolve_task_statuses,
    task_has_all_tags,
};
use self::hybrid::{
    AdrHybridCandidate, DocHybridCandidate, LearningHybridCandidate, blend_adr_hybrid_candidates,
    blend_adr_lexical_fallback, blend_doc_hybrid_candidates, blend_learning_hybrid_candidates,
    compare_global_hits_by_score, doc_search_candidate_limit, lexical_doc_hits_with_adrs,
    push_skip_note, warn_adr_hybrid_fallback, warn_doc_hybrid_fallback,
    warn_learning_hybrid_fallback,
};
use self::path_match::learning_scope_contains_path;

const DEFAULT_LIMIT: usize = 10;
const DOC_SEARCH_OVERFETCH: usize = 4;
const DOC_HYBRID_FALLBACK_NOTE: &str = "falling back to lexical doc search";
const ADR_HYBRID_FALLBACK_NOTE: &str = "falling back to lexical ADR search";
const LEARNING_HYBRID_FALLBACK_NOTE: &str = "falling back to lexical learning search";
const DOC_SEARCH_MIN_CANDIDATES: usize = DEFAULT_LIMIT * DOC_SEARCH_OVERFETCH;

#[cfg(test)]
thread_local! {
    static DOC_SEMANTIC_SEARCH_OVERRIDE:
        std::cell::RefCell<Option<Result<Vec<DocSemanticHit>, String>>> =
        const { std::cell::RefCell::new(None) };
    static ADR_SEMANTIC_SEARCH_OVERRIDE:
        std::cell::RefCell<Option<Result<Vec<AdrSemanticHit>, String>>> =
        const { std::cell::RefCell::new(None) };
    static LEARNING_SEMANTIC_SEARCH_OVERRIDE:
        std::cell::RefCell<Option<Result<Vec<LearningSemanticHit>, String>>> =
        const { std::cell::RefCell::new(None) };
}

#[derive(Debug, Clone, Copy)]
struct HybridSearchScope<'a> {
    params: &'a GlobalSearchParams,
    status_filters: &'a SearchStatusFilters,
    tag_filter: &'a [String],
    limit: usize,
}

impl OrbitRuntime {
    pub fn global_search(
        &self,
        params: GlobalSearchParams,
    ) -> Result<GlobalSearchResponse, OrbitError> {
        let limit = params.normalized_limit();
        let status_filters = SearchStatusFilters::parse(&params.status)?;
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
                model: None,
            })?;
            let results = related
                .results
                .into_iter()
                .map(semantic_hit_to_global)
                .collect();
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

        let mut branches = Vec::new();

        if params.kind.includes_tasks() {
            branches.push(self.task_branch(
                &params,
                &status_filters,
                query_owned.as_deref(),
                &tag_filter,
                limit,
            )?);
        }

        if params.kind.includes_docs() {
            if has_path {
                push_skip_note(
                    &mut notes,
                    "doc",
                    "--path is set; docs are not path-filtered yet",
                );
            } else {
                branches.push(self.doc_branch(
                    &params,
                    &status_filters,
                    query_owned.as_deref(),
                    &tag_filter,
                    limit,
                    &mut notes,
                )?);
            }
        }

        if params.kind.includes_adrs() {
            branches.push(self.adr_branch(
                &params,
                &status_filters,
                query_owned.as_deref(),
                &tag_filter,
                limit,
                &mut notes,
            )?);
        }

        if params.kind.includes_learnings() {
            branches.push(self.learning_branch(
                &params,
                &status_filters,
                query_owned.as_deref(),
                &tag_filter,
                limit,
                &mut notes,
            )?);
        }

        let results = merge_round_robin(branches, limit);
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
        status_filters: &SearchStatusFilters,
        query: Option<&str>,
        tag_filter: &[String],
        limit: usize,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let statuses = resolve_task_statuses(params, status_filters);

        let candidates = if params.hybrid
            && let Some(query) = query
        {
            let search = self.semantic_search(SemanticSearchParams {
                query: query.to_string(),
                limit: limit.saturating_mul(2).max(limit),
                field: None,
                kind: Some("task".to_string()),
                model: None,
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
        params: &GlobalSearchParams,
        status_filters: &SearchStatusFilters,
        query: Option<&str>,
        tag_filter: &[String],
        limit: usize,
        notes: &mut Vec<String>,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let _doc_status_active = status_filters.doc_active.unwrap_or(true);
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

        let docs_limit = doc_search_candidate_limit(limit);
        let docs = self.search_docs(query, Some(docs_limit), true)?;
        if params.hybrid {
            // ADR-0180: doc vectors are opt-in and fall back to lexical rather than failing user search.
            return self.hybrid_doc_hits(
                query,
                docs,
                HybridSearchScope {
                    params,
                    status_filters,
                    tag_filter,
                    limit,
                },
                notes,
            );
        }

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
                let score = result.score as f32;
                out.push(doc_result_to_global(result, "lexical", Some(score)));
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    fn hybrid_doc_hits(
        &self,
        query: &str,
        lexical_results: Vec<SearchResult>,
        scope: HybridSearchScope<'_>,
        notes: &mut Vec<String>,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let docs_limit = doc_search_candidate_limit(scope.limit);
        let mut lexical_docs = BTreeMap::<String, orbit_search::DocSearchResult>::new();
        let mut lexical_adrs = BTreeMap::<String, AdrHybridCandidate>::new();
        let adr_statuses = resolve_adr_statuses(scope.params, scope.status_filters);
        for result in lexical_results {
            match result {
                SearchResult::Doc(result) => {
                    if !scope.tag_filter.is_empty()
                        && !scope.tag_filter.iter().all(|tag| {
                            result
                                .record
                                .tags
                                .iter()
                                .any(|candidate| candidate.eq_ignore_ascii_case(tag))
                        })
                    {
                        continue;
                    }
                    lexical_docs.insert(result.record.path.clone(), result);
                }
                SearchResult::Adr(result) => {
                    if !adr_statuses.contains(&result.status) {
                        continue;
                    }
                    if scope.tag_filter.is_empty()
                        || adr_result_has_all_tags(&result, scope.tag_filter)
                    {
                        lexical_adrs.insert(
                            result.id.clone(),
                            AdrHybridCandidate {
                                hit: adr_result_to_global(result.clone(), "hybrid"),
                                lexical_score: Some(result.score as f32),
                                semantic_score: None,
                                semantic: None,
                            },
                        );
                    }
                }
            }
        }

        let semantic = match self.doc_semantic_hits(query, docs_limit) {
            Ok(result) if result.is_empty() => {
                warn_doc_hybrid_fallback(notes, "no doc embeddings found");
                return Ok(lexical_doc_hits_with_adrs(
                    lexical_docs,
                    lexical_adrs,
                    scope.limit,
                ));
            }
            Ok(result) => result,
            Err(error) => {
                warn_doc_hybrid_fallback(notes, &error.to_string());
                return Ok(lexical_doc_hits_with_adrs(
                    lexical_docs,
                    lexical_adrs,
                    scope.limit,
                ));
            }
        };

        let records = self
            .list_docs(None, None)?
            .into_iter()
            .map(|record| (record.path.clone(), record))
            .collect::<BTreeMap<_, _>>();
        let mut candidates = BTreeMap::<String, DocHybridCandidate>::new();
        for (path, result) in lexical_docs {
            candidates.insert(
                path,
                DocHybridCandidate {
                    hit: doc_result_to_global(result.clone(), "hybrid", None),
                    lexical_score: Some(result.score as f32),
                    semantic_score: None,
                    semantic: None,
                },
            );
        }
        for hit in semantic {
            let Some(record) = records.get(&hit.source_id) else {
                continue;
            };
            if !scope.tag_filter.is_empty() && !doc_has_all_tags(record, scope.tag_filter) {
                continue;
            }
            candidates
                .entry(hit.source_id.clone())
                .and_modify(|candidate| {
                    candidate.semantic_score = Some(hit.score);
                    candidate.semantic = Some(hit.clone());
                })
                .or_insert_with(|| DocHybridCandidate {
                    hit: GlobalSearchHit {
                        kind: "doc".to_string(),
                        source: "hybrid".to_string(),
                        id: None,
                        path: Some(record.path.clone()),
                        title: None,
                        summary: Some(record.frontmatter.summary.clone()),
                        status: Some(record.frontmatter.doc_type.as_str().to_string()),
                        best_field: None,
                        snippet: None,
                        score: None,
                        matched_by: None,
                    },
                    lexical_score: None,
                    semantic_score: Some(hit.score),
                    semantic: Some(hit),
                });
        }

        let weight = self.docs_search_config()?.semantic_weight;
        let mut ranked = blend_doc_hybrid_candidates(candidates.into_values().collect(), weight);
        let mut adr_ranked =
            self.hybrid_adr_hits_from_candidates(query, lexical_adrs, scope, notes)?;
        ranked.append(&mut adr_ranked);
        ranked.sort_by(compare_global_hits_by_score);
        ranked.truncate(scope.limit);
        Ok(ranked)
    }

    fn doc_semantic_hits(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<DocSemanticHit>, OrbitError> {
        #[cfg(test)]
        if let Some(result) = DOC_SEMANTIC_SEARCH_OVERRIDE.with(|cell| cell.borrow().clone()) {
            return result.map_err(OrbitError::Execution);
        }

        Ok(orbit_search::doc_semantic_search(
            &self.stores().semantic_vector,
            DocSemanticSearchParams {
                query: query.to_string(),
                limit,
                model: None,
            },
        )?
        .results)
    }

    fn adr_branch(
        &self,
        params: &GlobalSearchParams,
        status_filters: &SearchStatusFilters,
        query: Option<&str>,
        tag_filter: &[String],
        limit: usize,
        notes: &mut Vec<String>,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let lexical = self.adr_lexical_hits(params, status_filters, query, tag_filter, limit)?;
        if !params.hybrid {
            return Ok(lexical);
        }
        let Some(query) = query else {
            return Ok(lexical);
        };

        self.hybrid_adr_hits(
            query,
            lexical,
            HybridSearchScope {
                params,
                status_filters,
                tag_filter,
                limit,
            },
            notes,
        )
    }

    fn adr_lexical_hits(
        &self,
        params: &GlobalSearchParams,
        status_filters: &SearchStatusFilters,
        query: Option<&str>,
        tag_filter: &[String],
        limit: usize,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let statuses = resolve_adr_statuses(params, status_filters);
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

        let docs_limit = doc_search_candidate_limit(limit);
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
                out.push(adr_result_to_global(result, "lexical"));
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    fn hybrid_adr_hits(
        &self,
        query: &str,
        lexical: Vec<GlobalSearchHit>,
        scope: HybridSearchScope<'_>,
        notes: &mut Vec<String>,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let lexical_count = lexical.len();
        let mut lexical_adrs = BTreeMap::<String, AdrHybridCandidate>::new();
        for (idx, hit) in lexical.into_iter().enumerate() {
            let Some(id) = hit.id.clone() else {
                continue;
            };
            let lexical_score = hit.score.or(Some((lexical_count - idx) as f32));
            lexical_adrs.insert(
                id,
                AdrHybridCandidate {
                    hit: GlobalSearchHit {
                        source: "hybrid".to_string(),
                        ..hit
                    },
                    lexical_score,
                    semantic_score: None,
                    semantic: None,
                },
            );
        }

        self.hybrid_adr_hits_from_candidates(query, lexical_adrs, scope, notes)
    }

    fn hybrid_adr_hits_from_candidates(
        &self,
        query: &str,
        lexical_adrs: BTreeMap<String, AdrHybridCandidate>,
        scope: HybridSearchScope<'_>,
        notes: &mut Vec<String>,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let adr_limit = doc_search_candidate_limit(scope.limit);
        let semantic = match self.adr_semantic_hits(query, adr_limit) {
            Ok(result) if result.is_empty() => {
                warn_adr_hybrid_fallback(notes, "no ADR embeddings found");
                return Ok(blend_adr_lexical_fallback(lexical_adrs, scope.limit));
            }
            Ok(result) => result,
            Err(error) => {
                warn_adr_hybrid_fallback(notes, &error.to_string());
                return Ok(blend_adr_lexical_fallback(lexical_adrs, scope.limit));
            }
        };

        let statuses = resolve_adr_statuses(scope.params, scope.status_filters);
        let path = scope.params.path.as_deref();
        let mut candidates = lexical_adrs;
        for hit in semantic {
            let adr = match self.stores().adrs().get(&hit.source_id) {
                Ok(Some(adr)) => adr,
                Ok(None) | Err(_) => continue,
            };
            if !statuses.contains(&adr.status) {
                continue;
            }
            if !scope.tag_filter.is_empty() && !adr_has_all_tags(&adr, scope.tag_filter) {
                continue;
            }
            if let Some(path) = path
                && !orbit_search::adr_paths_contain_path(&adr.paths, path)?
            {
                continue;
            }

            candidates
                .entry(hit.source_id.clone())
                .and_modify(|candidate| {
                    candidate.semantic_score = Some(hit.score);
                    candidate.semantic = Some(hit.clone());
                })
                .or_insert_with(|| AdrHybridCandidate {
                    hit: adr_to_global_hit_with_source(adr, "hybrid", None),
                    lexical_score: None,
                    semantic_score: Some(hit.score),
                    semantic: Some(hit),
                });
        }

        let weight = self.adr_search_config()?.semantic_weight;
        let mut ranked = blend_adr_hybrid_candidates(candidates.into_values().collect(), weight);
        ranked.truncate(scope.limit);
        Ok(ranked)
    }

    fn adr_semantic_hits(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<AdrSemanticHit>, OrbitError> {
        #[cfg(test)]
        if let Some(result) = ADR_SEMANTIC_SEARCH_OVERRIDE.with(|cell| cell.borrow().clone()) {
            return result.map_err(OrbitError::Execution);
        }

        Ok(orbit_search::adr_semantic_search(
            &self.stores().semantic_vector,
            AdrSemanticSearchParams {
                query: query.to_string(),
                limit,
                model: None,
            },
        )?
        .results)
    }

    fn learning_branch(
        &self,
        params: &GlobalSearchParams,
        status_filters: &SearchStatusFilters,
        query: Option<&str>,
        tag_filter: &[String],
        limit: usize,
        notes: &mut Vec<String>,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let lexical =
            self.learning_lexical_hits(params, status_filters, query, tag_filter, limit)?;
        if !params.hybrid {
            return Ok(lexical);
        }
        let Some(query) = query else {
            return Ok(lexical);
        };

        self.hybrid_learning_hits(
            query,
            lexical,
            HybridSearchScope {
                params,
                status_filters,
                tag_filter,
                limit,
            },
            notes,
        )
    }

    fn learning_lexical_hits(
        &self,
        params: &GlobalSearchParams,
        status_filters: &SearchStatusFilters,
        query: Option<&str>,
        tag_filter: &[String],
        limit: usize,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let statuses = resolve_learning_statuses(params, status_filters);
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

    fn hybrid_learning_hits(
        &self,
        query: &str,
        lexical: Vec<GlobalSearchHit>,
        scope: HybridSearchScope<'_>,
        notes: &mut Vec<String>,
    ) -> Result<Vec<GlobalSearchHit>, OrbitError> {
        let learning_limit = doc_search_candidate_limit(scope.limit);
        let lexical_fallback = lexical.clone();
        let mut lexical_learnings = BTreeMap::<String, LearningHybridCandidate>::new();
        let lexical_count = lexical.len();
        for (idx, hit) in lexical.into_iter().enumerate() {
            let Some(id) = hit.id.clone() else {
                continue;
            };
            lexical_learnings.insert(
                id,
                LearningHybridCandidate {
                    hit: GlobalSearchHit {
                        source: "hybrid".to_string(),
                        ..hit
                    },
                    lexical_score: Some((lexical_count - idx) as f32),
                    semantic_score: None,
                    semantic: None,
                },
            );
        }

        let semantic = match self.learning_semantic_hits(query, learning_limit) {
            Ok(result) if result.is_empty() => {
                warn_learning_hybrid_fallback(notes, "no learning embeddings found");
                return Ok(lexical_fallback);
            }
            Ok(result) => result,
            Err(error) => {
                warn_learning_hybrid_fallback(notes, &error.to_string());
                return Ok(lexical_fallback);
            }
        };

        let statuses = resolve_learning_statuses(scope.params, scope.status_filters);
        let path = scope.params.path.as_deref();
        let mut candidates = lexical_learnings;
        for hit in semantic {
            let learning = match self.get_learning(&hit.source_id) {
                Ok(learning) => learning,
                Err(_) => continue,
            };
            if !statuses.contains(&learning.status) {
                continue;
            }
            if !scope.tag_filter.is_empty() && !learning_has_all_tags(&learning, scope.tag_filter) {
                continue;
            }
            if let Some(path) = path
                && !learning_scope_contains_path(&learning, path)?
            {
                continue;
            }

            candidates
                .entry(hit.source_id.clone())
                .and_modify(|candidate| {
                    candidate.semantic_score = Some(hit.score);
                    candidate.semantic = Some(hit.clone());
                })
                .or_insert_with(|| LearningHybridCandidate {
                    hit: GlobalSearchHit {
                        kind: "learning".to_string(),
                        source: "hybrid".to_string(),
                        id: Some(learning.id),
                        path: None,
                        title: None,
                        summary: Some(learning.summary),
                        status: Some(learning.status.as_str().to_string()),
                        best_field: None,
                        snippet: None,
                        score: None,
                        matched_by: None,
                    },
                    lexical_score: None,
                    semantic_score: Some(hit.score),
                    semantic: Some(hit),
                });
        }

        let weight = self.learning_search_config()?.semantic_weight;
        let mut ranked =
            blend_learning_hybrid_candidates(candidates.into_values().collect(), weight);
        ranked.truncate(scope.limit);
        Ok(ranked)
    }

    fn learning_semantic_hits(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<LearningSemanticHit>, OrbitError> {
        #[cfg(test)]
        if let Some(result) = LEARNING_SEMANTIC_SEARCH_OVERRIDE.with(|cell| cell.borrow().clone()) {
            return result.map_err(OrbitError::Execution);
        }

        Ok(orbit_search::learning_semantic_search(
            &self.stores().semantic_vector,
            LearningSemanticSearchParams {
                query: query.to_string(),
                limit,
                model: None,
            },
        )?
        .results)
    }
}

fn merge_round_robin(branches: Vec<Vec<GlobalSearchHit>>, limit: usize) -> Vec<GlobalSearchHit> {
    let mut queues = branches
        .into_iter()
        .filter(|branch| !branch.is_empty())
        .map(|branch| branch.into_iter().collect::<VecDeque<_>>())
        .collect::<Vec<_>>();
    let mut out = Vec::with_capacity(limit);

    while out.len() < limit && !queues.is_empty() {
        let mut index = 0;
        while index < queues.len() && out.len() < limit {
            if let Some(hit) = queues[index].pop_front() {
                out.push(hit);
            }
            if queues[index].is_empty() {
                queues.remove(index);
            } else {
                index += 1;
            }
        }
    }

    out
}
