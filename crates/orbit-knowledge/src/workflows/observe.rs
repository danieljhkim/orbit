//! Knowledge-graph read/observe workflow for CLI display.
//!
//! Consumed by the `orbit observe graph show|search|history` CLI subcommands.
//! Returns shapes optimized for human display (lineage names, structural
//! child/sibling counts, JSON payload) — distinct from the agent-facing
//! [`crate::commands::show`] / [`crate::commands::search`] surfaces which
//! return semantic content (imports, exports, source bodies).
//!
//! Returns [`KnowledgeError`]; host crates translate to `OrbitError` at the
//! edge.

// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use serde_json::{Value, json};

use crate::graph::navigator::GraphNodeRef;
use crate::service::GraphContextService;
use crate::workflows::{load_graph, parse_ref_name};
use crate::{GraphReadOptions, KnowledgeError};
use orbit_graph_extract::Selector;

pub const REMOVED_GRAPH_HISTORY_MESSAGE: &str = "Knowledge-graph task attribution has been removed. Use `git log --grep '[T<task-id>]'` for local forward lookup, and use `external_refs` for cross-engineer task references.";

#[derive(Debug, Clone)]
pub struct GraphShowOptions {
    pub data_root: std::path::PathBuf,
    pub selector: String,
    pub depth: usize,
    pub siblings: usize,
    pub children: usize,
    pub ref_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GraphShowOutput {
    pub payload: Value,
    pub selector: String,
    pub lineage_names: Vec<String>,
    pub details: GraphNodeDetails,
    pub siblings: Vec<String>,
    pub children: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum GraphNodeDetails {
    Dir {
        parent: Option<String>,
        dirs: usize,
        files: usize,
    },
    File {
        extension: Option<String>,
        parent: Option<String>,
        leaves: usize,
    },
    Leaf {
        kind: String,
        lines: Option<(u32, u32)>,
        parent: Option<String>,
        source: String,
    },
}

#[derive(Debug, Clone)]
pub struct GraphSearchOptions {
    pub data_root: std::path::PathBuf,
    pub query: String,
    pub node_types: Vec<String>,
    pub prefix: Option<String>,
    pub limit: usize,
    pub ref_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GraphSearchOutput {
    pub selectors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GraphHistoryOptions {
    pub selector: String,
    pub ref_name: Option<String>,
}

pub fn show_graph(options: GraphShowOptions) -> Result<GraphShowOutput, KnowledgeError> {
    let graph = load_graph(
        &options.data_root,
        options.ref_name.as_deref(),
        GraphReadOptions {
            hydrate_file_source: true,
            hydrate_leaf_source: true,
        },
    )?;
    let service = GraphContextService::new(&graph);

    let selector: Selector = options
        .selector
        .parse()
        .map_err(|error| KnowledgeError::invalid_data(format!("{error}")))?;

    let node = service
        .resolve_selector(&selector)
        .map_err(|error| KnowledgeError::invalid_data(error.to_string()))?;

    let context = service
        .bounded_context(node.id(), options.depth, options.siblings, options.children)
        .map_err(|error| KnowledgeError::knowledge_unavailable(error.to_string()))?;

    Ok(show_output_from_context(&service, &context))
}

pub fn search_graph(options: GraphSearchOptions) -> Result<GraphSearchOutput, KnowledgeError> {
    let graph = load_graph(
        &options.data_root,
        options.ref_name.as_deref(),
        Default::default(),
    )?;
    let service = GraphContextService::new(&graph);

    let type_refs: Vec<&str> = options.node_types.iter().map(String::as_str).collect();
    let node_types = if type_refs.is_empty() {
        None
    } else {
        Some(type_refs.as_slice())
    };

    let selectors = service
        .search(
            &options.query,
            node_types,
            options.prefix.as_deref(),
            None,
            options.limit,
        )
        .into_iter()
        .map(|node| service.selector_for_node(node))
        .collect();

    Ok(GraphSearchOutput { selectors })
}

pub fn history_graph(options: GraphHistoryOptions) -> Result<(), KnowledgeError> {
    let _selector: Selector = options
        .selector
        .parse()
        .map_err(|error| KnowledgeError::invalid_data(format!("{error}")))?;
    parse_ref_name(options.ref_name)?;

    Err(KnowledgeError::invalid_data(
        REMOVED_GRAPH_HISTORY_MESSAGE.to_string(),
    ))
}

pub fn node_context_payload(
    service: &GraphContextService<'_>,
    context: &crate::service::NodeContext<'_>,
) -> Value {
    let node = context.node;

    let lineage: Vec<String> = context
        .lineage
        .iter()
        .map(|node| service.selector_for_node(*node))
        .collect();
    let siblings: Vec<String> = context
        .siblings
        .iter()
        .map(|node| service.selector_for_node(*node))
        .collect();
    let children: Vec<String> = context
        .children
        .iter()
        .map(|node| service.selector_for_node(*node))
        .collect();

    let mut value = json!({
        "selector": service.selector_for_node(node),
        "lineage": lineage,
        "siblings": siblings,
        "children": children,
    });

    match node {
        GraphNodeRef::Leaf(leaf) => {
            let obj = value.as_object_mut().expect("node context payload object");
            obj.insert("source".to_string(), json!(leaf.source));
            obj.insert("lines".to_string(), json!([leaf.start_line, leaf.end_line]));
        }
        GraphNodeRef::File(file) => {
            let obj = value.as_object_mut().expect("node context payload object");
            if !file.source.is_empty() {
                obj.insert("source".to_string(), json!(file.source));
            }
            if let Some(source_blob_hash) = file.source_blob_hash.as_ref() {
                obj.insert("source_blob_hash".to_string(), json!(source_blob_hash));
            }
            if !file.imports.is_empty() {
                obj.insert("imports".to_string(), json!(file.imports));
            }
            if !file.exports.is_empty() {
                obj.insert("exports".to_string(), json!(file.exports));
            }
            if !file.re_exports.is_empty() {
                obj.insert("re_exports".to_string(), json!(file.re_exports));
            }
        }
        GraphNodeRef::Dir(_) => {}
    }

    value
}

fn show_output_from_context(
    service: &GraphContextService<'_>,
    context: &crate::service::NodeContext<'_>,
) -> GraphShowOutput {
    let node = context.node;
    let selector = service.selector_for_node(node);
    let lineage_names = context
        .lineage
        .iter()
        .map(|node| node.base().name.clone())
        .chain(std::iter::once(node.base().name.clone()))
        .collect();
    let siblings = context
        .siblings
        .iter()
        .map(|node| service.selector_for_node(*node))
        .collect();
    let children = context
        .children
        .iter()
        .map(|node| service.selector_for_node(*node))
        .collect();
    let details = node_details(service, node);
    let payload = node_context_payload(service, context);

    GraphShowOutput {
        payload,
        selector,
        lineage_names,
        details,
        siblings,
        children,
    }
}

fn node_details(service: &GraphContextService<'_>, node: GraphNodeRef<'_>) -> GraphNodeDetails {
    let parent = node
        .parent_id()
        .and_then(|parent_id| service.navigator().get_node(parent_id).ok())
        .map(|parent| service.selector_for_node(parent));

    match node {
        GraphNodeRef::Dir(dir) => GraphNodeDetails::Dir {
            parent,
            dirs: dir.dir_children.len(),
            files: dir.file_children.len(),
        },
        GraphNodeRef::File(file) => GraphNodeDetails::File {
            extension: file.extension.clone(),
            parent,
            leaves: file.leaf_children.len(),
        },
        GraphNodeRef::Leaf(leaf) => GraphNodeDetails::Leaf {
            kind: leaf.kind.to_string(),
            lines: leaf.start_line.zip(leaf.end_line),
            parent,
            source: leaf.source.clone(),
        },
    }
}
