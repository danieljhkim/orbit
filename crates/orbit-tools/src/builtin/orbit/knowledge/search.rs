use orbit_common::types::{OrbitError, ToolParam, ToolSchema, optional_raw_string};
use orbit_knowledge::GraphReadOptions;
use orbit_knowledge::graph::navigator::GraphNodeRef;
use orbit_knowledge::graph::object_store::{GraphObjectStore, resolve_graph_read_target};
use orbit_knowledge::graph::{GraphIndexReader, GraphIndexSearchRow};
use orbit_knowledge::service::{GraphContextService, SearchHit};
use regex::Regex;
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeSearchTool;

const DEFAULT_LIMIT: usize = 20;
const DEFAULT_RANKING_HEADROOM: usize = 10;
const DEFAULT_RANKING_HARD_CAP: usize = 5_000;
const SOURCE_REGEX_UNBOUNDED_LIMIT_MAX: usize = 200;

impl Tool for OrbitKnowledgeSearchTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.search".to_string(),
            description: format!(
                "Use when you need to locate selectors by name/path/source regex. Prefer over grep when text hits lose node type or source-context filters. Default ranking keeps at most {DEFAULT_RANKING_HARD_CAP} candidates before ranking."
            ),
            parameters: vec![
                ToolParam {
                    name: "query".to_string(),
                    description: "Name/path.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "type".to_string(),
                    description: "dir/file/symbol.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "kind".to_string(),
                    description: "Kind.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "prefix".to_string(),
                    description: "Prefix.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "source_regex".to_string(),
                    description:
                        "Rust regex; empty query needs prefix/limit<=200; adds matched_lines."
                            .to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "limit".to_string(),
                    description: "Limit.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "include_non_code".to_string(),
                    description: "Docs/configs.".to_string(),
                    param_type: "boolean".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "format".to_string(),
                    description: "structured/selectors.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                super::super::graph_ref_param(),
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let query = optional_raw_string(&input, "query")?
            .map(|value| value.trim().to_string())
            .unwrap_or_default();
        let node_type = super::super::optional_string(&input, "type")?;
        let kind_filter = super::super::optional_string(&input, "kind")?;
        let prefix = super::super::optional_string(&input, "prefix")?;
        let source_regex_text = super::super::optional_string(&input, "source_regex")?;
        let limit_provided = input.get("limit").is_some();
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_LIMIT as u64) as usize;
        validate_source_regex_bound(
            &query,
            prefix.as_deref(),
            limit,
            limit_provided,
            &source_regex_text,
        )?;
        let source_regex = compile_source_regex(source_regex_text.as_deref())?;
        let has_source_regex = source_regex.is_some();
        let include_non_code = input
            .get("include_non_code")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let format = super::super::optional_string(&input, "format")?;
        let use_selectors = format.as_deref() == Some("selectors");
        let use_default_ranking =
            node_type.is_none() && kind_filter.is_none() && prefix.is_none() && !has_source_regex;

        if use_default_ranking
            && let Some(payload) = try_default_search_via_sql_index(
                ctx,
                &input,
                &query,
                include_non_code,
                use_selectors,
                limit,
            )?
        {
            return Ok(payload);
        }

        let graph = super::load_graph_for_read(
            ctx,
            &input,
            GraphReadOptions {
                hydrate_file_source: has_source_regex,
                hydrate_leaf_source: has_source_regex,
            },
        )?;
        let svc = GraphContextService::new(&graph);

        let type_strs: Vec<&str> = node_type.iter().map(String::as_str).collect();
        let node_types = if type_strs.is_empty() {
            None
        } else {
            Some(type_strs.as_slice())
        };
        let search_limit = if use_default_ranking {
            // Bound retained service hits while still ranking a wider candidate pool down to `limit`.
            default_ranking_search_limit(limit)
        } else {
            limit
        };
        let candidate_scan_limit =
            if has_source_regex && prefix.is_none() && query.trim().is_empty() {
                Some(SOURCE_REGEX_UNBOUNDED_LIMIT_MAX)
            } else {
                None
            };
        let (service_total, hits) = svc
            .search_hits_with_total_bounded(
                &query,
                node_types,
                prefix.as_deref(),
                kind_filter.as_deref(),
                source_regex.as_ref(),
                search_limit,
                candidate_scan_limit,
            )
            .map_err(|error| {
                OrbitError::InvalidInput(format!(
                    "`source_regex` scanned more than {} source candidates; provide `prefix` or non-empty `query` to narrow the search",
                    error.limit
                ))
            })?;
        let (total, hits) = if use_default_ranking {
            let nodes = hits.into_iter().map(|hit| hit.node).collect();
            let ranked = rank_default_search_results(nodes, include_non_code);
            let total = ranked.len();
            let limited = ranked
                .into_iter()
                .take(limit)
                .map(|node| SearchHit {
                    node,
                    matched_lines: Vec::new(),
                })
                .collect::<Vec<_>>();
            (total, limited)
        } else {
            (service_total, hits)
        };

        if use_selectors {
            let selectors: Vec<String> = hits
                .iter()
                .map(|hit| svc.selector_for_node(hit.node))
                .collect();
            Ok(json!(selectors))
        } else {
            let items: Vec<Value> = hits
                .into_iter()
                .map(|hit| {
                    let node = hit.node;
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

                    let mut obj = json!({
                        "selector": svc.selector_for_node(node),
                        "name": node.base().name,
                        "kind": kind,
                    });
                    if let Some(file) = file {
                        obj["file"] = json!(file);
                    }
                    if has_source_regex {
                        obj["matched_lines"] = json!(
                            hit.matched_lines
                                .into_iter()
                                .map(|line| json!({
                                    "line_number": line.line_number,
                                    "snippet": line.snippet,
                                }))
                                .collect::<Vec<_>>()
                        );
                    }
                    obj
                })
                .collect();
            Ok(json!({
                "total": total,
                "results": items,
            }))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefaultSearchFastPath {
    ExactName,
    PathPrefix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefaultSearchExecutionPath {
    SqlExactName,
    SqlPathPrefix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SqlDefaultSearchOutcome {
    path: DefaultSearchExecutionPath,
    total: usize,
    rows: Vec<GraphIndexSearchRow>,
}

/// Classifies default-mode graph search queries into the only shapes backed by
/// SQLite today:
///
/// - Empty/browse queries, wildcard queries (`*` or `?`), regex/fuzzy-looking
///   queries, and multi-token non-path queries stay on the graph scan path.
/// - Exact-name queries are one token with no slash or wildcard; they use
///   `WHERE name_lower = ?` and fall back to scan if the exact index probe has
///   no rows so substring misses still work.
/// - Path-prefix queries contain `/`, have no trailing wildcard, and use
///   `WHERE location_lower LIKE ? ESCAPE '\'` with literal `%`, `_`, and `\`
///   escaped before appending the prefix `%`.
fn classify_default_search_query(query: &str) -> Option<DefaultSearchFastPath> {
    let query = query.trim();
    if query.is_empty() || query.contains('*') || query.contains('?') {
        return None;
    }

    if query.contains('/') {
        return Some(DefaultSearchFastPath::PathPrefix);
    }

    if query.split_whitespace().count() != 1 || looks_like_regex_or_fuzzy_query(query) {
        return None;
    }

    Some(DefaultSearchFastPath::ExactName)
}

fn looks_like_regex_or_fuzzy_query(query: &str) -> bool {
    query.chars().any(|ch| {
        matches!(
            ch,
            '^' | '$' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '~'
        )
    })
}

fn try_default_search_via_sql_index(
    ctx: &ToolContext,
    input: &Value,
    query: &str,
    include_non_code: bool,
    use_selectors: bool,
    limit: usize,
) -> Result<Option<Value>, OrbitError> {
    let Some(outcome) =
        try_default_search_rows_via_sql_index(ctx, input, query, include_non_code, limit)?
    else {
        return Ok(None);
    };

    if use_selectors {
        let selectors: Vec<String> = outcome.rows.iter().map(selector_for_search_row).collect();
        return Ok(Some(json!(selectors)));
    }

    let items: Vec<Value> = outcome.rows.into_iter().map(search_row_payload).collect();
    Ok(Some(json!({
        "total": outcome.total,
        "results": items,
    })))
}

fn try_default_search_rows_via_sql_index(
    ctx: &ToolContext,
    input: &Value,
    query: &str,
    include_non_code: bool,
    limit: usize,
) -> Result<Option<SqlDefaultSearchOutcome>, OrbitError> {
    if classify_default_search_query(query).is_none() {
        return Ok(None);
    }

    let Some(reader) = open_current_graph_index(ctx, input)? else {
        return Ok(None);
    };

    search_default_rows_with_graph_index(&reader, query, include_non_code, limit)
}

fn search_default_rows_with_graph_index(
    reader: &GraphIndexReader,
    query: &str,
    include_non_code: bool,
    limit: usize,
) -> Result<Option<SqlDefaultSearchOutcome>, OrbitError> {
    let Some(path) = classify_default_search_query(query) else {
        return Ok(None);
    };

    let query_lower = query.to_lowercase();
    let (execution_path, rows) = match path {
        DefaultSearchFastPath::ExactName => {
            let rows = reader.search_exact_name(&query_lower).map_err(|error| {
                OrbitError::Execution(format!("query graph sqlite exact-name search: {error}"))
            })?;
            if rows.is_empty() {
                return Ok(None);
            }
            (DefaultSearchExecutionPath::SqlExactName, rows)
        }
        DefaultSearchFastPath::PathPrefix => (
            DefaultSearchExecutionPath::SqlPathPrefix,
            reader
                .search_location_prefix(&query_lower)
                .map_err(|error| {
                    OrbitError::Execution(format!(
                        "query graph sqlite location-prefix search: {error}"
                    ))
                })?,
        ),
    };

    let ranked = rank_sql_default_search_results(rows, include_non_code);
    let total = ranked.len();
    Ok(Some(SqlDefaultSearchOutcome {
        path: execution_path,
        total,
        rows: ranked.into_iter().take(limit).collect(),
    }))
}

fn open_current_graph_index(
    ctx: &ToolContext,
    input: &Value,
) -> Result<Option<GraphIndexReader>, OrbitError> {
    let knowledge_dir = super::write::resolve_knowledge_dir(ctx, input)?;
    let explicit_ref = super::super::optional_string(input, "ref")?;

    if explicit_ref.is_none()
        && !super::has_explicit_knowledge_dir(input)
        && let Some(workspace_root) = ctx.workspace_root.as_deref()
    {
        let _ = orbit_knowledge::pipeline::ensure_fresh(&knowledge_dir, workspace_root);
    }

    let read_target =
        match resolve_graph_read_target(ctx.workspace_root.as_deref(), explicit_ref.as_deref()) {
            Ok(target) => target,
            Err(_) => return Ok(None),
        };
    let graph_store = GraphObjectStore::new(knowledge_dir.join("graph"));
    if graph_store
        .prepare_refs_layout(read_target.default.as_ref())
        .is_err()
    {
        return Ok(None);
    }
    let resolved =
        match graph_store.resolve_ref(&read_target.requested, read_target.fallback.as_ref()) {
            Ok(resolved) => resolved,
            Err(_) => return Ok(None),
        };

    match GraphIndexReader::open_current(
        graph_store.graph_sqlite_index_path(),
        &resolved.current_ref.root_graph_hash,
    ) {
        Ok(reader) => Ok(reader),
        Err(_) => Ok(None),
    }
}

fn rank_sql_default_search_results(
    rows: Vec<GraphIndexSearchRow>,
    include_non_code: bool,
) -> Vec<GraphIndexSearchRow> {
    let mut ranked: Vec<(usize, usize, GraphIndexSearchRow)> = rows
        .into_iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let rank = default_search_rank_for_row(&row);
            if !include_non_code && rank == 2 {
                return None;
            }
            Some((rank, index, row))
        })
        .collect();

    ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    ranked.into_iter().map(|(_, _, row)| row).collect()
}

fn default_search_rank_for_row(row: &GraphIndexSearchRow) -> usize {
    match row.node_type.as_str() {
        "leaf" if row.kind.as_deref().is_some_and(is_code_symbol_kind) => 0,
        "leaf" => 2,
        "file" | "dir" => path_search_rank(&row.location),
        _ => 2,
    }
}

fn search_row_payload(row: GraphIndexSearchRow) -> Value {
    let kind = kind_for_search_row(&row);
    let file = file_for_search_row(&row);
    let mut obj = json!({
        "selector": selector_for_search_row(&row),
        "name": row.name,
        "kind": kind,
    });
    if let Some(file) = file {
        obj["file"] = json!(file);
    }
    obj
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

fn validate_source_regex_bound(
    query: &str,
    prefix: Option<&str>,
    limit: usize,
    limit_provided: bool,
    source_regex: &Option<String>,
) -> Result<(), OrbitError> {
    if source_regex.is_none() {
        return Ok(());
    }
    if prefix.is_some() || !query.trim().is_empty() {
        return Ok(());
    }
    if limit_provided && limit <= SOURCE_REGEX_UNBOUNDED_LIMIT_MAX {
        return Ok(());
    }

    Err(OrbitError::InvalidInput(format!(
        "`source_regex` without `prefix` or non-empty `query` requires explicit `limit` <= {SOURCE_REGEX_UNBOUNDED_LIMIT_MAX}"
    )))
}

fn compile_source_regex(source_regex: Option<&str>) -> Result<Option<Regex>, OrbitError> {
    source_regex
        .map(|pattern| {
            Regex::new(pattern).map_err(|error| {
                OrbitError::InvalidInput(format!("invalid `source_regex`: {error}"))
            })
        })
        .transpose()
}

fn default_ranking_search_limit(limit: usize) -> usize {
    limit
        .saturating_mul(DEFAULT_RANKING_HEADROOM)
        .min(DEFAULT_RANKING_HARD_CAP)
}

fn rank_default_search_results<'a>(
    nodes: Vec<GraphNodeRef<'a>>,
    include_non_code: bool,
) -> Vec<GraphNodeRef<'a>> {
    let mut ranked: Vec<(usize, usize, GraphNodeRef<'a>)> = nodes
        .into_iter()
        .enumerate()
        .filter_map(|(index, node)| {
            let rank = default_search_rank(node);
            if !include_non_code && rank == 2 {
                return None;
            }
            Some((rank, index, node))
        })
        .collect();

    ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    ranked.into_iter().map(|(_, _, node)| node).collect()
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
            | "impl"
            | "class"
            | "interface"
            | "field"
            | "module"
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

#[cfg(test)]
mod tests {
    use orbit_knowledge::graph::GraphIndexReader;
    use orbit_knowledge::graph::nodes::{
        BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode,
    };
    use orbit_knowledge::graph::object_store::GraphObjectStore;
    use std::time::Instant;

    use super::*;

    #[test]
    fn default_ranking_search_uses_finite_capped_bound_for_large_graph() {
        const REQUESTED_LIMIT: usize = 10;
        const FIXTURE_LEAF_COUNT: usize = 1_000;

        let graph = graph_with_matching_leaves(FIXTURE_LEAF_COUNT);
        let service = GraphContextService::new(&graph);
        let search_limit = default_ranking_search_limit(REQUESTED_LIMIT);

        assert!(search_limit <= DEFAULT_RANKING_HARD_CAP);
        assert_eq!(search_limit, REQUESTED_LIMIT * DEFAULT_RANKING_HEADROOM);

        let (_total, hits) =
            service.search_hits_with_total("fixture", None, None, None, None, search_limit);

        assert_eq!(hits.len(), search_limit);
    }

    #[test]
    fn default_ranking_search_limit_saturates_at_hard_cap() {
        let over_cap_limit = DEFAULT_RANKING_HARD_CAP / DEFAULT_RANKING_HEADROOM + 1;

        assert_eq!(
            default_ranking_search_limit(over_cap_limit),
            DEFAULT_RANKING_HARD_CAP
        );
    }

    #[test]
    fn exact_name_query_uses_sql_index_when_rows_exist() {
        let graph = graph_for_sql_search_tests();
        let (_temp_dir, reader) = index_reader_for_graph(&graph);

        let outcome = search_default_rows_with_graph_index(&reader, "UniqueSymbol", false, 20)
            .expect("sql search")
            .expect("sql outcome");

        assert_eq!(outcome.path, DefaultSearchExecutionPath::SqlExactName);
        assert_eq!(
            selectors_from_search_rows(&outcome.rows),
            fallback_selectors(&graph, "UniqueSymbol", false, 20)
        );
    }

    #[test]
    fn path_prefix_query_uses_escaped_sql_like() {
        let graph = graph_for_sql_search_tests();
        let (_temp_dir, reader) = index_reader_for_graph(&graph);

        let outcome = search_default_rows_with_graph_index(&reader, "src/special%_dir/", false, 20)
            .expect("sql search")
            .expect("sql outcome");

        assert_eq!(outcome.path, DefaultSearchExecutionPath::SqlPathPrefix);
        assert_eq!(
            selectors_from_search_rows(&outcome.rows),
            vec![
                "symbol:src/special%_dir/mod.rs#EscapedSymbol:function".to_string(),
                "dir:src/special%_dir".to_string(),
                "file:src/special%_dir/mod.rs".to_string(),
            ]
        );
        assert_eq!(
            selectors_from_search_rows(&outcome.rows),
            fallback_selectors(&graph, "src/special%_dir/", false, 20)
        );
    }

    #[test]
    fn substring_and_regex_shapes_stay_on_scan_path() {
        let graph = graph_for_sql_search_tests();
        let (_temp_dir, reader) = index_reader_for_graph(&graph);

        let exact_probe_miss = search_default_rows_with_graph_index(&reader, "nique", false, 20)
            .expect("substring search after exact miss");
        let substring = search_default_rows_with_graph_index(&reader, "Unique Symbol", false, 20)
            .expect("substring search");
        let regex = search_default_rows_with_graph_index(&reader, "^UniqueSymbol$", false, 20)
            .expect("regex search");

        assert!(exact_probe_miss.is_none());
        assert!(substring.is_none());
        assert!(regex.is_none());
        assert_eq!(
            fallback_selectors(&graph, "nique", false, 20),
            vec!["symbol:src/core/main.rs#UniqueSymbol:function".to_string()]
        );
    }

    #[test]
    fn missing_or_stale_index_falls_back_to_scan_for_all_query_shapes() {
        let graph = graph_for_sql_search_tests();
        let (temp_dir, _reader) = index_reader_for_graph(&graph);
        let store = GraphObjectStore::new(temp_dir.path().join("graph"));

        assert!(
            GraphIndexReader::open_current(store.graph_sqlite_index_path(), "stale-root")
                .expect("open stale index")
                .is_none()
        );
        std::fs::remove_file(store.graph_sqlite_index_path()).expect("delete sqlite index");
        assert!(
            GraphIndexReader::open_current(store.graph_sqlite_index_path(), "missing-root")
                .expect("open missing index")
                .is_none()
        );

        for query in [
            "UniqueSymbol",
            "src/core/",
            "Unique Symbol",
            "^UniqueSymbol$",
        ] {
            assert_eq!(
                fallback_selectors(&graph, query, false, 20),
                fallback_selectors_when_sql_unavailable(None, &graph, query, false, 20),
                "missing index fallback changed output for {query}"
            );
        }
    }

    #[test]
    fn sql_fast_path_honors_user_limit() {
        let graph = graph_with_repeated_name_leaves(25, "CommonSymbol");
        let (_temp_dir, reader) = index_reader_for_graph(&graph);

        let outcome = search_default_rows_with_graph_index(&reader, "CommonSymbol", false, 10)
            .expect("sql search")
            .expect("sql outcome");

        assert_eq!(outcome.path, DefaultSearchExecutionPath::SqlExactName);
        assert!(outcome.rows.len() <= 10);
        assert_eq!(outcome.rows.len(), 10);
    }

    #[test]
    #[ignore = "manual latency observation; run when refreshing graph search performance numbers"]
    fn exact_name_sql_path_outperforms_scan_on_10k_leaf_fixture() {
        let graph = graph_with_matching_leaves(10_000);
        let (_temp_dir, reader) = index_reader_for_graph(&graph);

        let sql_started = Instant::now();
        let sql = search_default_rows_with_graph_index(&reader, "fixture_9999", false, 10)
            .expect("sql search")
            .expect("sql outcome");
        let sql_elapsed = sql_started.elapsed();

        let scan_started = Instant::now();
        let scan = fallback_selectors(&graph, "fixture_9999", false, 10);
        let scan_elapsed = scan_started.elapsed();

        assert_eq!(sql.rows.len(), 1);
        assert_eq!(scan.len(), 1);
        assert!(
            scan_elapsed.as_nanos() >= sql_elapsed.as_nanos() * 10,
            "expected SQL at least 10x faster; sql={sql_elapsed:?} scan={scan_elapsed:?}"
        );
    }

    fn graph_with_matching_leaves(leaf_count: usize) -> CodebaseGraphV1 {
        let root_id = "dir:.".to_string();
        let file_id = "file:src/fixture.rs".to_string();
        let mut leaf_ids = Vec::with_capacity(leaf_count);
        let mut leaves = Vec::with_capacity(leaf_count);

        for index in 0..leaf_count {
            let name = format!("fixture_{index}");
            let leaf_id = format!("symbol:src/fixture.rs#{name}:function");
            leaf_ids.push(leaf_id.clone());
            leaves.push(LeafNode {
                base: base_node(
                    &leaf_id,
                    &name,
                    &format!("src/fixture.rs#{name}"),
                    "rust",
                    Some(&file_id),
                ),
                kind: LeafKind::Function,
                source: String::new(),
                source_blob_hash: None,
                source_hash: None,
                file_hash_at_capture: None,
                history: Vec::new(),
                input_signature: Vec::new(),
                output_signature: Vec::new(),
                start_line: Some((index + 1) as u32),
                end_line: Some((index + 1) as u32),
                children: Vec::new(),
            });
        }

        CodebaseGraphV1 {
            root_dir_id: root_id.clone(),
            dirs: vec![DirNode {
                base: base_node(&root_id, ".", ".", "", None),
                dir_children: Vec::new(),
                file_children: vec![file_id.clone()],
            }],
            files: vec![FileNode {
                base: base_node(
                    &file_id,
                    "fixture.rs",
                    "src/fixture.rs",
                    "rust",
                    Some(&root_id),
                ),
                extension: Some("rs".to_string()),
                source_blob_hash: None,
                source: String::new(),
                imports: Vec::new(),
                exports: Vec::new(),
                re_exports: Vec::new(),
                leaf_children: leaf_ids,
            }],
            leaves,
        }
    }

    fn graph_for_sql_search_tests() -> CodebaseGraphV1 {
        let root_id = "dir:.".to_string();
        let src_dir_id = "dir:src".to_string();
        let core_dir_id = "dir:src/core".to_string();
        let special_dir_id = "dir:src/special%_dir".to_string();
        let lookalike_dir_id = "dir:src/specialABdir".to_string();
        let core_file_id = "file:src/core/main.rs".to_string();
        let special_file_id = "file:src/special%_dir/mod.rs".to_string();
        let lookalike_file_id = "file:src/specialABdir/mod.rs".to_string();
        let unique_leaf_id = "symbol:src/core/main.rs#UniqueSymbol:function".to_string();
        let escaped_leaf_id = "symbol:src/special%_dir/mod.rs#EscapedSymbol:function".to_string();
        let lookalike_leaf_id =
            "symbol:src/specialABdir/mod.rs#LookalikeSymbol:function".to_string();

        CodebaseGraphV1 {
            root_dir_id: root_id.clone(),
            dirs: vec![
                DirNode {
                    base: base_node(&root_id, ".", ".", "", None),
                    dir_children: vec![src_dir_id.clone()],
                    file_children: Vec::new(),
                },
                DirNode {
                    base: base_node(&src_dir_id, "src", "src/", "", Some(&root_id)),
                    dir_children: vec![
                        core_dir_id.clone(),
                        special_dir_id.clone(),
                        lookalike_dir_id.clone(),
                    ],
                    file_children: Vec::new(),
                },
                DirNode {
                    base: base_node(&core_dir_id, "core", "src/core/", "", Some(&src_dir_id)),
                    dir_children: Vec::new(),
                    file_children: vec![core_file_id.clone()],
                },
                DirNode {
                    base: base_node(
                        &special_dir_id,
                        "special%_dir",
                        "src/special%_dir/",
                        "",
                        Some(&src_dir_id),
                    ),
                    dir_children: Vec::new(),
                    file_children: vec![special_file_id.clone()],
                },
                DirNode {
                    base: base_node(
                        &lookalike_dir_id,
                        "specialABdir",
                        "src/specialABdir/",
                        "",
                        Some(&src_dir_id),
                    ),
                    dir_children: Vec::new(),
                    file_children: vec![lookalike_file_id.clone()],
                },
            ],
            files: vec![
                FileNode {
                    base: base_node(
                        &core_file_id,
                        "main.rs",
                        "src/core/main.rs",
                        "rust",
                        Some(&core_dir_id),
                    ),
                    extension: Some("rs".to_string()),
                    source_blob_hash: None,
                    source: String::new(),
                    imports: Vec::new(),
                    exports: Vec::new(),
                    re_exports: Vec::new(),
                    leaf_children: vec![unique_leaf_id.clone()],
                },
                FileNode {
                    base: base_node(
                        &special_file_id,
                        "mod.rs",
                        "src/special%_dir/mod.rs",
                        "rust",
                        Some(&special_dir_id),
                    ),
                    extension: Some("rs".to_string()),
                    source_blob_hash: None,
                    source: String::new(),
                    imports: Vec::new(),
                    exports: Vec::new(),
                    re_exports: Vec::new(),
                    leaf_children: vec![escaped_leaf_id.clone()],
                },
                FileNode {
                    base: base_node(
                        &lookalike_file_id,
                        "mod.rs",
                        "src/specialABdir/mod.rs",
                        "rust",
                        Some(&lookalike_dir_id),
                    ),
                    extension: Some("rs".to_string()),
                    source_blob_hash: None,
                    source: String::new(),
                    imports: Vec::new(),
                    exports: Vec::new(),
                    re_exports: Vec::new(),
                    leaf_children: vec![lookalike_leaf_id.clone()],
                },
            ],
            leaves: vec![
                leaf_node(
                    &unique_leaf_id,
                    "UniqueSymbol",
                    "src/core/main.rs#UniqueSymbol",
                    Some(&core_file_id),
                    1,
                ),
                leaf_node(
                    &escaped_leaf_id,
                    "EscapedSymbol",
                    "src/special%_dir/mod.rs#EscapedSymbol",
                    Some(&special_file_id),
                    2,
                ),
                leaf_node(
                    &lookalike_leaf_id,
                    "LookalikeSymbol",
                    "src/specialABdir/mod.rs#LookalikeSymbol",
                    Some(&lookalike_file_id),
                    3,
                ),
            ],
        }
    }

    fn graph_with_repeated_name_leaves(leaf_count: usize, name: &str) -> CodebaseGraphV1 {
        let root_id = "dir:.".to_string();
        let dir_id = "dir:src/limit".to_string();
        let mut file_ids = Vec::with_capacity(leaf_count);
        let mut files = Vec::with_capacity(leaf_count);
        let mut leaves = Vec::with_capacity(leaf_count);

        for index in 0..leaf_count {
            let file_id = format!("file:src/limit/file_{index:05}.rs");
            let location = format!("src/limit/file_{index:05}.rs");
            let leaf_id = format!("symbol:{location}#{name}:function");
            file_ids.push(file_id.clone());
            files.push(FileNode {
                base: base_node(
                    &file_id,
                    &format!("file_{index:05}.rs"),
                    &location,
                    "rust",
                    Some(&dir_id),
                ),
                extension: Some("rs".to_string()),
                source_blob_hash: None,
                source: String::new(),
                imports: Vec::new(),
                exports: Vec::new(),
                re_exports: Vec::new(),
                leaf_children: vec![leaf_id.clone()],
            });
            leaves.push(leaf_node(
                &leaf_id,
                name,
                &format!("{location}#{name}"),
                Some(&file_id),
                (index + 1) as u32,
            ));
        }

        CodebaseGraphV1 {
            root_dir_id: root_id.clone(),
            dirs: vec![
                DirNode {
                    base: base_node(&root_id, ".", ".", "", None),
                    dir_children: vec![dir_id.clone()],
                    file_children: Vec::new(),
                },
                DirNode {
                    base: base_node(&dir_id, "limit", "src/limit/", "", Some(&root_id)),
                    dir_children: Vec::new(),
                    file_children: file_ids,
                },
            ],
            files,
            leaves,
        }
    }

    fn index_reader_for_graph(graph: &CodebaseGraphV1) -> (tempfile::TempDir, GraphIndexReader) {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let store = GraphObjectStore::new(temp_dir.path().join("graph"));
        let current_ref = store.write_graph(graph).expect("write graph");
        let reader = GraphIndexReader::open_current(
            store.graph_sqlite_index_path(),
            &current_ref.root_graph_hash,
        )
        .expect("open sqlite index")
        .expect("current sqlite index");
        (temp_dir, reader)
    }

    fn fallback_selectors(
        graph: &CodebaseGraphV1,
        query: &str,
        include_non_code: bool,
        limit: usize,
    ) -> Vec<String> {
        let service = GraphContextService::new(graph);
        let search_limit = default_ranking_search_limit(limit);
        let (_total, hits) =
            service.search_hits_with_total(query, None, None, None, None, search_limit);
        let nodes = hits.into_iter().map(|hit| hit.node).collect();
        rank_default_search_results(nodes, include_non_code)
            .into_iter()
            .take(limit)
            .map(|node| service.selector_for_node(node))
            .collect()
    }

    fn fallback_selectors_when_sql_unavailable(
        reader: Option<&GraphIndexReader>,
        graph: &CodebaseGraphV1,
        query: &str,
        include_non_code: bool,
        limit: usize,
    ) -> Vec<String> {
        if let Some(reader) = reader
            && let Ok(Some(outcome)) =
                search_default_rows_with_graph_index(reader, query, include_non_code, limit)
        {
            return selectors_from_search_rows(&outcome.rows);
        }
        fallback_selectors(graph, query, include_non_code, limit)
    }

    fn selectors_from_search_rows(rows: &[GraphIndexSearchRow]) -> Vec<String> {
        rows.iter().map(selector_for_search_row).collect()
    }

    fn leaf_node(
        id: &str,
        name: &str,
        location: &str,
        parent_id: Option<&str>,
        line: u32,
    ) -> LeafNode {
        LeafNode {
            base: base_node(id, name, location, "rust", parent_id),
            kind: LeafKind::Function,
            source: String::new(),
            source_blob_hash: None,
            source_hash: None,
            file_hash_at_capture: None,
            history: Vec::new(),
            input_signature: Vec::new(),
            output_signature: Vec::new(),
            start_line: Some(line),
            end_line: Some(line),
            children: Vec::new(),
        }
    }

    fn base_node(
        id: &str,
        name: &str,
        location: &str,
        language: &str,
        parent_id: Option<&str>,
    ) -> BaseNodeFields {
        BaseNodeFields {
            id: id.to_string(),
            identity_key: id.to_string(),
            object_hash: None,
            name: name.to_string(),
            location: location.to_string(),
            language: language.to_string(),
            description: String::new(),
            parent_id: parent_id.map(str::to_string),
            is_locked: false,
            lineage_locked: false,
            lock_owner: None,
            lock_reason: String::new(),
        }
    }
}
