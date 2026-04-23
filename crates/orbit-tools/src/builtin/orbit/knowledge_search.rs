use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::graph::navigator::GraphNodeRef;
use orbit_knowledge::service::GraphContextService;
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeSearchTool;

const DEFAULT_LIMIT: usize = 20;

impl Tool for OrbitKnowledgeSearchTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.search".to_string(),
            description: "Use when you need to locate selectors. Prefer over grep when repeated names make raw hits ambiguous. Behavior: without `type`, `kind`, or `prefix`, code symbols rank first and non-code stays hidden unless `include_non_code` is true.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "query".to_string(),
                    description: "Name or path substring.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "type".to_string(),
                    description: "`dir`, `file`, or `symbol`.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "kind".to_string(),
                    description: "Leaf kind.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "prefix".to_string(),
                    description: "Only this path prefix.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "limit".to_string(),
                    description: "Max results.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "include_non_code".to_string(),
                    description: "Include doc/config hits too.".to_string(),
                    param_type: "boolean".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "format".to_string(),
                    description: "`structured` or `selectors`.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                super::graph_ref_param(),
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let query = super::optional_string(&input, "query")?.unwrap_or_default();
        let node_type = super::optional_string(&input, "type")?;
        let kind_filter = super::optional_string(&input, "kind")?;
        let prefix = super::optional_string(&input, "prefix")?;
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_LIMIT as u64) as usize;
        let include_non_code = input
            .get("include_non_code")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let format = super::optional_string(&input, "format")?;
        let use_selectors = format.as_deref() == Some("selectors");
        let use_default_ranking = node_type.is_none() && kind_filter.is_none() && prefix.is_none();

        let graph = super::load_graph_for_read(ctx, &input)?;
        let svc = GraphContextService::new(&graph);

        let type_strs: Vec<&str> = node_type.iter().map(String::as_str).collect();
        let node_types = if type_strs.is_empty() {
            None
        } else {
            Some(type_strs.as_slice())
        };
        let search_limit = if use_default_ranking {
            usize::MAX
        } else {
            limit
        };
        let (service_total, nodes) = svc.search_with_total(
            &query,
            node_types,
            prefix.as_deref(),
            kind_filter.as_deref(),
            search_limit,
        );
        let (total, nodes) = if use_default_ranking {
            let ranked = rank_default_search_results(nodes, include_non_code);
            let total = ranked.len();
            let limited = ranked.into_iter().take(limit).collect::<Vec<_>>();
            (total, limited)
        } else {
            (service_total, nodes)
        };

        if use_selectors {
            let selectors: Vec<String> = nodes.iter().map(|n| svc.selector_for_node(*n)).collect();
            Ok(json!(selectors))
        } else {
            let items: Vec<Value> = nodes
                .into_iter()
                .map(|node| {
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
