use regex::Regex;

use crate::commands::{GraphCommandContext, fuzzy};
use crate::graph::navigator::GraphNodeRef;
use crate::graph::{GraphIndexSearchRow, GraphReadOptions};
use crate::service::{GraphContextService, MatchedLine, SearchHit};
use crate::{KnowledgeError, graph::nodes::CodebaseGraphV1};

const DEFAULT_RANKING_HEADROOM: usize = 10;
pub(crate) const DEFAULT_RANKING_HARD_CAP: usize = 5_000;
pub const SOURCE_REGEX_UNBOUNDED_LIMIT_MAX: usize = 200;

#[derive(Debug, Clone)]
pub struct SearchInput {
    pub context: GraphCommandContext,
    pub query: String,
    pub node_type: Option<String>,
    pub kind_filter: Option<String>,
    pub prefix: Option<String>,
    pub source_regex: Option<Regex>,
    pub include_non_code: bool,
    pub allow_fuzzy: bool,
    pub limit: usize,
}

#[derive(Debug, Clone)]
pub struct DefaultSearchInput<'a> {
    pub graph: &'a CodebaseGraphV1,
    pub query: &'a str,
    pub limit: usize,
    pub include_non_code: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub total: usize,
    pub hits: Vec<SearchResultItem>,
    pub used_index: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResultItem {
    pub selector: String,
    pub name: String,
    pub kind: String,
    pub file: Option<String>,
    pub matched_lines: Vec<MatchedLine>,
    pub match_kind: Option<String>,
    pub score: Option<f32>,
}

pub fn run(input: SearchInput) -> Result<SearchResult, KnowledgeError> {
    let has_source_regex = input.source_regex.is_some();
    let allow_fuzzy = input.allow_fuzzy && !has_source_regex && !input.query.trim().is_empty();
    let use_default_ranking = input.node_type.is_none()
        && input.kind_filter.is_none()
        && input.prefix.is_none()
        && !has_source_regex;

    if use_default_ranking
        && let Some(result) = try_default_search_via_sql_index(
            &input.context,
            &input.query,
            input.include_non_code,
            input.limit,
        )?
    {
        return maybe_fuzzy_fallback(&input, result, allow_fuzzy, None);
    }

    let graph = input.context.read_graph(GraphReadOptions {
        hydrate_file_source: has_source_regex,
        hydrate_leaf_source: has_source_regex,
    })?;
    let svc = GraphContextService::new(&graph);

    let type_strs: Vec<&str> = input.node_type.iter().map(String::as_str).collect();
    let node_types = if type_strs.is_empty() {
        None
    } else {
        Some(type_strs.as_slice())
    };
    let use_exact_symbol_definition_ranking = should_rank_exact_symbol_definitions(
        &input.query,
        input.node_type.as_deref(),
        input.kind_filter.as_deref(),
        has_source_regex,
    );
    let search_limit = if use_default_ranking || use_exact_symbol_definition_ranking {
        default_ranking_search_limit(input.limit)
    } else {
        input.limit
    };
    let candidate_scan_limit =
        if has_source_regex && input.prefix.is_none() && input.query.trim().is_empty() {
            Some(SOURCE_REGEX_UNBOUNDED_LIMIT_MAX)
        } else {
            None
        };
    let (service_total, hits) = svc
        .search_hits_with_total_bounded(
            &input.query,
            node_types,
            input.prefix.as_deref(),
            input.kind_filter.as_deref(),
            input.source_regex.as_ref(),
            search_limit,
            candidate_scan_limit,
        )
        .map_err(|error| {
            KnowledgeError::invalid_data(format!(
                "`source_regex` scanned more than {} source candidates; provide `prefix` or non-empty `query` to narrow the search",
                error.limit
            ))
        })?;

    if use_default_ranking {
        let nodes = hits.into_iter().map(|hit| hit.node).collect();
        let ranked = rank_default_search_results(nodes, input.include_non_code, &input.query);
        let total = ranked.len();
        let hits = ranked
            .into_iter()
            .take(input.limit)
            .map(|node| search_item_for_node(&svc, node, Vec::new()))
            .collect();
        let result = SearchResult {
            total,
            hits,
            used_index: false,
        };
        maybe_fuzzy_fallback(&input, result, allow_fuzzy, Some(&graph))
    } else {
        let hits = if use_exact_symbol_definition_ranking {
            rank_exact_symbol_definition_hits(hits, &input.query)
                .into_iter()
                .take(input.limit)
                .collect()
        } else {
            hits
        };
        let result = SearchResult {
            total: service_total,
            hits: hits
                .into_iter()
                .map(|hit| search_item_for_hit(&svc, hit))
                .collect(),
            used_index: false,
        };
        maybe_fuzzy_fallback(&input, result, allow_fuzzy, Some(&graph))
    }
}

fn maybe_fuzzy_fallback(
    input: &SearchInput,
    result: SearchResult,
    allow_fuzzy: bool,
    graph: Option<&CodebaseGraphV1>,
) -> Result<SearchResult, KnowledgeError> {
    if !allow_fuzzy || result.total != 0 {
        return Ok(result);
    }

    let owned_graph;
    let graph = if let Some(graph) = graph {
        graph
    } else {
        owned_graph = input.context.read_graph(GraphReadOptions {
            hydrate_file_source: false,
            hydrate_leaf_source: false,
        })?;
        &owned_graph
    };

    let hits = fuzzy::fuzzy_name_candidates(graph, &input.query, input.limit)
        .into_iter()
        .map(|candidate| SearchResultItem {
            selector: candidate.selector,
            name: candidate.name,
            kind: candidate.kind,
            file: candidate.file,
            matched_lines: Vec::new(),
            match_kind: Some("fuzzy".to_string()),
            score: Some(candidate.score),
        })
        .collect::<Vec<_>>();
    let total = hits.len();

    Ok(SearchResult {
        total,
        hits,
        used_index: false,
    })
}

pub fn default_search(input: DefaultSearchInput<'_>) -> Result<SearchResult, KnowledgeError> {
    let svc = GraphContextService::new(input.graph);
    let search_limit = default_ranking_search_limit(input.limit);
    let (_total, hits) =
        svc.search_hits_with_total(input.query, None, None, None, None, search_limit);
    let nodes = hits.into_iter().map(|hit| hit.node).collect();
    let ranked = rank_default_search_results(nodes, input.include_non_code, input.query);
    let total = ranked.len();
    let hits = ranked
        .into_iter()
        .take(input.limit)
        .map(|node| search_item_for_node(&svc, node, Vec::new()))
        .collect();
    Ok(SearchResult {
        total,
        hits,
        used_index: false,
    })
}

fn try_default_search_via_sql_index(
    context: &GraphCommandContext,
    query: &str,
    include_non_code: bool,
    limit: usize,
) -> Result<Option<SearchResult>, KnowledgeError> {
    let Some(reader) = context.open_current_graph_index()? else {
        return Ok(None);
    };

    let query_lower = query.trim().to_lowercase();
    let scan_cap = default_ranking_search_limit(limit);
    let rows = reader
        .search_substring(&query_lower, scan_cap)
        .map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "query graph sqlite substring search: {error}"
            ))
        })?;

    let ranked = rank_sql_default_search_results(rows, include_non_code, query);
    let total = ranked.len();
    let hits = ranked
        .into_iter()
        .take(limit)
        .map(search_item_for_row)
        .collect();
    Ok(Some(SearchResult {
        total,
        hits,
        used_index: true,
    }))
}

fn search_item_for_hit(svc: &GraphContextService<'_>, hit: SearchHit<'_>) -> SearchResultItem {
    search_item_for_node(svc, hit.node, hit.matched_lines)
}

fn search_item_for_node(
    svc: &GraphContextService<'_>,
    node: GraphNodeRef<'_>,
    matched_lines: Vec<MatchedLine>,
) -> SearchResultItem {
    let kind = match node {
        GraphNodeRef::Dir(_) => "dir".to_string(),
        GraphNodeRef::File(_) => "file".to_string(),
        GraphNodeRef::Leaf(leaf) => leaf.kind.to_string(),
    };
    let file = match node {
        GraphNodeRef::Leaf(leaf) => leaf
            .base
            .location
            .split_once('#')
            .map(|(path, _)| path.to_string()),
        GraphNodeRef::File(file) => Some(file.base.location.clone()),
        GraphNodeRef::Dir(_) => None,
    };

    SearchResultItem {
        selector: svc.selector_for_node(node),
        name: node.base().name.clone(),
        kind,
        file,
        matched_lines,
        match_kind: None,
        score: None,
    }
}

fn search_item_for_row(row: GraphIndexSearchRow) -> SearchResultItem {
    let selector = selector_for_search_row(&row);
    let kind = kind_for_search_row(&row);
    let file = file_for_search_row(&row);
    SearchResultItem {
        selector,
        name: row.name,
        kind,
        file,
        matched_lines: Vec::new(),
        match_kind: None,
        score: None,
    }
}

fn rank_sql_default_search_results(
    rows: Vec<GraphIndexSearchRow>,
    include_non_code: bool,
    query: &str,
) -> Vec<GraphIndexSearchRow> {
    let query = query.trim();
    let mut ranked: Vec<(usize, usize, usize, GraphIndexSearchRow)> = rows
        .into_iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let rank = default_search_rank_for_row(&row);
            if !include_non_code && rank == 2 {
                return None;
            }
            Some((
                exact_symbol_definition_rank_for_row(&row, query),
                rank,
                index,
                row,
            ))
        })
        .collect();

    ranked.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    ranked.into_iter().map(|(_, _, _, row)| row).collect()
}

fn default_search_rank_for_row(row: &GraphIndexSearchRow) -> usize {
    match row.node_type.as_str() {
        "leaf" if row.kind.as_deref().is_some_and(is_code_symbol_kind) => 0,
        "leaf" => 2,
        "file" | "dir" => path_search_rank(&row.location),
        _ => 2,
    }
}

fn selector_for_search_row(row: &GraphIndexSearchRow) -> String {
    row.selector
        .clone()
        .unwrap_or_else(|| match row.node_type.as_str() {
            "dir" => {
                let path = row.location.trim_end_matches('/');
                format!("dir:{path}")
            }
            "file" => format!("file:{}", row.location),
            "leaf" => {
                let kind = row.kind.as_deref().unwrap_or_default();
                format!("symbol:{}:{kind}", row.location)
            }
            _ => row.id.clone(),
        })
}

fn kind_for_search_row(row: &GraphIndexSearchRow) -> String {
    match row.node_type.as_str() {
        "leaf" => row.kind.clone().unwrap_or_else(|| "symbol".to_string()),
        other => other.to_string(),
    }
}

fn file_for_search_row(row: &GraphIndexSearchRow) -> Option<String> {
    match row.node_type.as_str() {
        "leaf" => row
            .location
            .split_once('#')
            .map(|(path, _)| path.to_string()),
        "file" => Some(row.location.clone()),
        _ => None,
    }
}

fn default_ranking_search_limit(limit: usize) -> usize {
    limit
        .saturating_mul(DEFAULT_RANKING_HEADROOM)
        .min(DEFAULT_RANKING_HARD_CAP)
}

fn should_rank_exact_symbol_definitions(
    query: &str,
    node_type: Option<&str>,
    kind_filter: Option<&str>,
    has_source_regex: bool,
) -> bool {
    !has_source_regex
        && kind_filter.is_none()
        && !query.trim().is_empty()
        && node_type.is_none_or(|node_type| node_type == "symbol")
}

fn rank_exact_symbol_definition_hits<'a>(
    hits: Vec<SearchHit<'a>>,
    query: &str,
) -> Vec<SearchHit<'a>> {
    let query = query.trim();
    let mut ranked: Vec<(usize, usize, SearchHit<'a>)> = hits
        .into_iter()
        .enumerate()
        .map(|(index, hit)| {
            (
                exact_symbol_definition_rank_for_node(hit.node, query),
                index,
                hit,
            )
        })
        .collect();

    ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    ranked.into_iter().map(|(_, _, hit)| hit).collect()
}

fn rank_default_search_results<'a>(
    nodes: Vec<GraphNodeRef<'a>>,
    include_non_code: bool,
    query: &str,
) -> Vec<GraphNodeRef<'a>> {
    let query = query.trim();
    let mut ranked: Vec<(usize, usize, usize, GraphNodeRef<'a>)> = nodes
        .into_iter()
        .enumerate()
        .filter_map(|(index, node)| {
            let rank = default_search_rank(node);
            if !include_non_code && rank == 2 {
                return None;
            }
            Some((
                exact_symbol_definition_rank_for_node(node, query),
                rank,
                index,
                node,
            ))
        })
        .collect();

    ranked.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    ranked.into_iter().map(|(_, _, _, node)| node).collect()
}

fn default_search_rank(node: GraphNodeRef<'_>) -> usize {
    match node {
        GraphNodeRef::Leaf(leaf) if is_code_symbol_kind(leaf.kind.to_string().as_str()) => 0,
        GraphNodeRef::Leaf(_) => 2,
        GraphNodeRef::File(file) => path_search_rank(&file.base.location),
        GraphNodeRef::Dir(dir) => path_search_rank(&dir.base.location),
    }
}

fn path_search_rank(path: &str) -> usize {
    if is_non_code_path(path) { 2 } else { 1 }
}

fn is_code_symbol_kind(kind: &str) -> bool {
    matches!(
        kind,
        "function"
            | "method"
            | "struct"
            | "trait"
            | "enum"
            | "type"
            | "type_alias"
            | "impl"
            | "class"
            | "interface"
            | "field"
            | "module"
    )
}

fn exact_symbol_definition_rank_for_node(node: GraphNodeRef<'_>, query: &str) -> usize {
    match node {
        GraphNodeRef::Leaf(leaf)
            if leaf.base.name == query
                && is_preferred_exact_definition_kind(leaf.kind.to_string().as_str()) =>
        {
            0
        }
        _ => 1,
    }
}

fn exact_symbol_definition_rank_for_row(row: &GraphIndexSearchRow, query: &str) -> usize {
    match (row.node_type.as_str(), row.kind.as_deref()) {
        ("leaf", Some(kind)) if row.name == query && is_preferred_exact_definition_kind(kind) => 0,
        _ => 1,
    }
}

fn is_preferred_exact_definition_kind(kind: &str) -> bool {
    matches!(
        kind,
        "trait" | "struct" | "enum" | "type" | "type_alias" | "function" | "module"
    )
}

fn is_non_code_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    matches!(
        extension.as_str(),
        "md" | "txt" | "rst" | "adoc" | "yaml" | "yml" | "toml" | "json" | "jsonc" | "csv" | "tsv"
    ) || lower.starts_with("docs/")
}

