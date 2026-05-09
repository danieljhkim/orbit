use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::graph::object_store::{GraphObjectStore, resolve_graph_read_target};
use orbit_knowledge::graph::{GraphIndexNodeRow, GraphIndexReader, GraphNode};
use orbit_knowledge::service::GraphContextService;
use orbit_knowledge::{GraphReadOptions, Selector};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeShowTool;

impl Tool for OrbitKnowledgeShowTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.show".to_string(),
            description: "Use when you need one resolved node with nearby context. Prefer over grep when you need lineage, children, siblings, or source.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Node selector.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "depth".to_string(),
                    description: "Ancestor depth.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "siblings".to_string(),
                    description: "Max siblings.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "children".to_string(),
                    description: "Max children.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                super::super::graph_ref_param(),
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let selector_str = super::super::required_string(&input, &["selector"], "selector")?;
        let selector: Selector = selector_str
            .parse()
            .map_err(|e| OrbitError::InvalidInput(format!("{e}")))?;

        let depth = input.get("depth").and_then(Value::as_u64).unwrap_or(2) as usize;
        let max_siblings = input.get("siblings").and_then(Value::as_u64).unwrap_or(3) as usize;
        let max_children = input.get("children").and_then(Value::as_u64).unwrap_or(5) as usize;

        if let Some(payload) =
            try_show_via_sql_index(ctx, &input, &selector, depth, max_siblings, max_children)?
        {
            return Ok(payload);
        }

        let graph = super::load_graph_for_read(
            ctx,
            &input,
            GraphReadOptions {
                hydrate_file_source: true,
                hydrate_leaf_source: true,
            },
        )?;
        let svc = GraphContextService::new(&graph);

        let node = svc
            .resolve_selector(&selector)
            .map_err(|e| OrbitError::InvalidInput(e.to_string()))?;

        let node_ctx = svc
            .bounded_context(node.id(), depth, max_siblings, max_children)
            .map_err(|e| OrbitError::Execution(e.to_string()))?;

        Ok(crate::graph::node_context_payload(&svc, &node_ctx))
    }
}

fn try_show_via_sql_index(
    ctx: &ToolContext,
    input: &Value,
    selector: &Selector,
    depth: usize,
    max_siblings: usize,
    max_children: usize,
) -> Result<Option<Value>, OrbitError> {
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
    let reader = match GraphIndexReader::open_current(
        graph_store.graph_sqlite_index_path(),
        &resolved.current_ref.root_graph_hash,
    ) {
        Ok(Some(reader)) => reader,
        Ok(None) | Err(_) => return Ok(None),
    };

    let Some(row) = reader
        .find_node_by_selector(&selector.to_string())
        .map_err(|error| OrbitError::Execution(format!("query graph sqlite index: {error}")))?
    else {
        return Ok(None);
    };

    let node = graph_store
        .read_node_by_object_hash(
            &row.id,
            &row.node_type,
            &row.object_hash,
            GraphReadOptions {
                hydrate_file_source: true,
                hydrate_leaf_source: true,
            },
        )
        .map_err(|error| OrbitError::Execution(format!("read graph node: {error}")))?;
    let lineage = reader
        .lineage_for(row.parent_id.as_deref(), depth)
        .map_err(|error| OrbitError::Execution(format!("query graph sqlite lineage: {error}")))?;
    let siblings = if let Some(parent_id) = row.parent_id.as_deref() {
        reader
            .children_of(parent_id, max_siblings.saturating_add(1))
            .map_err(|error| {
                OrbitError::Execution(format!("query graph sqlite siblings: {error}"))
            })?
            .into_iter()
            .filter(|sibling| sibling.id != row.id)
            .take(max_siblings)
            .collect()
    } else {
        Vec::new()
    };
    let children = reader
        .children_of(&row.id, max_children)
        .map_err(|error| OrbitError::Execution(format!("query graph sqlite children: {error}")))?;

    Ok(Some(sql_node_context_payload(
        &node, &lineage, &siblings, &children,
    )))
}

fn sql_node_context_payload(
    node: &GraphNode,
    lineage: &[GraphIndexNodeRow],
    siblings: &[GraphIndexNodeRow],
    children: &[GraphIndexNodeRow],
) -> Value {
    let lineage: Vec<String> = lineage.iter().map(selector_for_index_row).collect();
    let siblings: Vec<String> = siblings.iter().map(selector_for_index_row).collect();
    let children: Vec<String> = children.iter().map(selector_for_index_row).collect();

    let mut value = json!({
        "selector": selector_for_graph_node(node),
        "lineage": lineage,
        "siblings": siblings,
        "children": children,
    });

    match node {
        GraphNode::Leaf(leaf) => {
            let obj = value.as_object_mut().expect("node context payload object");
            obj.insert("source".to_string(), json!(leaf.source));
            obj.insert("lines".to_string(), json!([leaf.start_line, leaf.end_line]));
        }
        GraphNode::File(file) => {
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
        GraphNode::Dir(_) => {}
    }

    value
}

fn selector_for_graph_node(node: &GraphNode) -> String {
    match node {
        GraphNode::Dir(dir) => {
            let path = dir.base.location.trim_end_matches('/');
            format!("dir:{path}")
        }
        GraphNode::File(file) => format!("file:{}", file.base.location),
        GraphNode::Leaf(leaf) => format!("symbol:{}:{}", leaf.base.location, leaf.kind),
    }
}

fn selector_for_index_row(row: &GraphIndexNodeRow) -> String {
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
