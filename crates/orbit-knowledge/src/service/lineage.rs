//! Lineage pack rendering: bounded context packs for selectors.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};

use crate::error::KnowledgeError;
use crate::graph::navigator::GraphNodeRef;
use crate::graph::nodes::CodebaseGraphV1;
use crate::graph::object_store::GraphObjectStore;
use crate::selector::Selector;
use crate::service::GraphContextService;

/// Options for rendering a lineage pack.
pub struct LineagePackOptions {
    /// How many ancestors to include per node.
    pub depth: usize,
    /// Max siblings per node.
    pub siblings: usize,
    /// Max children per node.
    pub children: usize,
    /// Output format.
    pub format: LineageFormat,
}

impl Default for LineagePackOptions {
    fn default() -> Self {
        Self {
            depth: 2,
            siblings: 2,
            children: 4,
            format: LineageFormat::Json,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineageFormat {
    Json,
    Markdown,
}

/// Render a lineage pack for the given selectors.
///
/// Loads the graph from `knowledge_dir`, resolves each selector,
/// gathers bounded context, and renders as JSON or Markdown.
pub fn render_lineage_pack(
    knowledge_dir: &Path,
    selectors: &[Selector],
    options: &LineagePackOptions,
) -> Result<String, KnowledgeError> {
    let store = GraphObjectStore::new(knowledge_dir.join("graph"));
    let graph = store.read_graph()?;
    render_lineage_pack_from_graph(&graph, selectors, options)
}

/// Render a lineage pack from an already-loaded graph.
pub fn render_lineage_pack_from_graph(
    graph: &CodebaseGraphV1,
    selectors: &[Selector],
    options: &LineagePackOptions,
) -> Result<String, KnowledgeError> {
    let svc = GraphContextService::new(graph);
    let (resolved, unresolved) = svc.resolve_many(selectors);

    // Build context for each resolved node
    let mut selections = Vec::new();
    let mut context_nodes: BTreeMap<String, Value> = BTreeMap::new();

    for node in &resolved {
        let sel_str = svc.selector_for_node(*node);
        let ctx =
            svc.bounded_context(node.id(), options.depth, options.siblings, options.children)?;

        let lineage_sels: Vec<String> = ctx
            .lineage
            .iter()
            .map(|n| svc.selector_for_node(*n))
            .collect();

        selections.push(json!({
            "selector": sel_str,
            "node_type": node_type_str(*node),
            "lineage": lineage_sels,
        }));

        // Add the node itself and its context nodes to the map
        add_node_to_map(&svc, *node, &ctx, &mut context_nodes);
    }

    let pack = json!({
        "repo": {
            "root_selector": format!("dir:{}", graph.root_dir_id),
            "requested_count": selectors.len(),
            "resolved_count": resolved.len(),
            "unresolved": unresolved,
            "dir_count": graph.dirs.len(),
            "file_count": graph.files.len(),
            "leaf_count": graph.leaves.len(),
        },
        "selections": selections,
        "nodes": context_nodes,
    });

    match options.format {
        LineageFormat::Json => serde_json::to_string_pretty(&pack)
            .map_err(|e| KnowledgeError::invalid_data(format!("json render: {e}"))),
        LineageFormat::Markdown => Ok(render_markdown(&pack)),
    }
}

fn node_type_str(node: GraphNodeRef<'_>) -> &'static str {
    match node {
        GraphNodeRef::Dir(_) => "dir",
        GraphNodeRef::File(_) => "file",
        GraphNodeRef::Leaf(_) => "symbol",
    }
}

fn add_node_to_map(
    svc: &GraphContextService<'_>,
    node: GraphNodeRef<'_>,
    ctx: &super::NodeContext<'_>,
    map: &mut BTreeMap<String, Value>,
) {
    let sel = svc.selector_for_node(node);
    if map.contains_key(&sel) {
        return;
    }

    let parent_sel = node
        .parent_id()
        .and_then(|pid| svc.navigator().get_node(pid).ok())
        .map(|p| svc.selector_for_node(p));

    let sibling_sels: Vec<String> = ctx
        .siblings
        .iter()
        .map(|n| svc.selector_for_node(*n))
        .collect();

    let child_sels: Vec<String> = ctx
        .children
        .iter()
        .map(|n| svc.selector_for_node(*n))
        .collect();

    let details = build_node_details(node);

    map.insert(
        sel,
        json!({
            "type": node_type_str(node),
            "name": node.base().name,
            "location": node.location(),
            "parent": parent_sel,
            "siblings": sibling_sels,
            "children": child_sels,
            "details": details,
        }),
    );

    // Also add lineage nodes (compact, without their own context)
    for lineage_node in &ctx.lineage {
        let lsel = svc.selector_for_node(*lineage_node);
        map.entry(lsel).or_insert_with(|| {
            json!({
                "type": node_type_str(*lineage_node),
                "name": lineage_node.base().name,
                "location": lineage_node.location(),
            })
        });
    }
}

fn build_node_details(node: GraphNodeRef<'_>) -> Value {
    match node {
        GraphNodeRef::Dir(d) => json!({
            "dir_children_count": d.dir_children.len(),
            "file_children_count": d.file_children.len(),
        }),
        GraphNodeRef::File(f) => json!({
            "extension": f.extension,
            "imports": f.imports,
            "exports": f.exports,
            "leaf_count": f.leaf_children.len(),
        }),
        GraphNodeRef::Leaf(l) => json!({
            "kind": l.kind.to_string(),
            "source": l.source,
            "start_line": l.start_line,
            "end_line": l.end_line,
        }),
    }
}

fn render_markdown(pack: &Value) -> String {
    let mut out = String::new();

    // Header
    if let Some(repo) = pack.get("repo") {
        out.push_str("# Knowledge Pack\n\n");
        if let Some(n) = repo.get("resolved_count") {
            out.push_str(&format!("Resolved: {n} selectors\n\n"));
        }
    }

    // Selections
    if let Some(Value::Array(selections)) = pack.get("selections") {
        out.push_str("## Selections\n\n");
        for sel in selections {
            if let Some(Value::String(s)) = sel.get("selector") {
                out.push_str(&format!("- `{s}`\n"));
            }
        }
        out.push('\n');
    }

    // Nodes
    if let Some(Value::Object(nodes)) = pack.get("nodes") {
        out.push_str("## Nodes\n\n");
        for (selector, info) in nodes {
            out.push_str(&format!("### `{selector}`\n\n"));

            if let Some(Value::String(name)) = info.get("name") {
                out.push_str(&format!("**Name:** {name}\n"));
            }
            if let Some(Value::String(loc)) = info.get("location") {
                out.push_str(&format!("**Location:** {loc}\n"));
            }

            if let Some(details) = info.get("details")
                && let Some(Value::String(source)) = details.get("source")
                && !source.is_empty()
            {
                out.push_str("\n```\n");
                out.push_str(source);
                if !source.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("```\n");
            }
            out.push('\n');
        }
    }

    out
}
