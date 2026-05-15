use serde_json::Value;

use crate::commands::GraphCommandContext;
use crate::graph::object_store::{GraphObjectStore, resolve_graph_read_target};
use crate::graph::{CodebaseGraphV1, GraphIndexNodeRow, GraphNode, GraphReadOptions, LeafKind};
use crate::service::{GraphContextService, NodeContext};
use crate::{KnowledgeError, Selector};

/// Diagnostic suggestions are intentionally capped so failed lookups stay cheap
/// and payloads remain small for agent callers.
const DID_YOU_MEAN_LIMIT: usize = 5;

#[derive(Debug, Clone)]
pub struct ShowInput {
    pub context: GraphCommandContext,
    pub selector: String,
    pub depth: usize,
    pub max_siblings: usize,
    pub max_children: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShowResult {
    pub selector: String,
    pub lineage: Vec<String>,
    pub siblings: Vec<String>,
    pub children: Vec<String>,
    pub details: ShowNodeDetails,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShowNodeDetails {
    Dir,
    File {
        source: Option<String>,
        source_blob_hash: Option<String>,
        imports: Vec<String>,
        exports: Vec<String>,
        re_exports: Vec<Value>,
    },
    Leaf {
        source: String,
        start_line: Option<u32>,
        end_line: Option<u32>,
    },
}

pub fn run(input: ShowInput) -> Result<ShowResult, KnowledgeError> {
    let selector: Selector = input
        .selector
        .parse()
        .map_err(|error| KnowledgeError::invalid_data(format!("{error}")))?;

    if let Some(result) = try_show_via_sql_index(
        &input.context,
        &selector,
        input.depth,
        input.max_siblings,
        input.max_children,
    )? {
        return Ok(result);
    }

    let graph = input.context.read_graph(GraphReadOptions {
        hydrate_file_source: true,
        hydrate_leaf_source: true,
    })?;
    let svc = GraphContextService::new(&graph);
    let node = match svc.resolve_selector(&selector) {
        Ok(node) => node,
        Err(error) => {
            return Err(invalid_selector_resolution_error(&graph, &selector, error));
        }
    };
    let context = svc
        .bounded_context(
            node.id(),
            input.depth,
            input.max_siblings,
            input.max_children,
        )
        .map_err(|error| KnowledgeError::knowledge_unavailable(error.to_string()))?;

    Ok(result_from_context(&svc, &context))
}

fn invalid_selector_resolution_error(
    graph: &CodebaseGraphV1,
    selector: &Selector,
    error: KnowledgeError,
) -> KnowledgeError {
    let reason = error.to_string();
    KnowledgeError::invalid_data_with_suggestions(
        reason,
        did_you_mean_for_unresolved_selector(graph, selector),
    )
}

fn did_you_mean_for_unresolved_selector(
    graph: &CodebaseGraphV1,
    selector: &Selector,
) -> Vec<String> {
    let Selector::Symbol { path, symbol, kind } = selector else {
        return Vec::new();
    };
    if kind != "method" {
        return Vec::new();
    }
    let Some((container, requested_method)) = symbol.rsplit_once("::") else {
        return Vec::new();
    };
    if requested_method.is_empty()
        || !file_resolves(graph, path)
        || !containing_context_resolves(graph, path, container)
    {
        return Vec::new();
    }

    let requested_method_lower = requested_method.to_ascii_lowercase();
    let mut candidates = graph
        .leaves
        .iter()
        .filter(|leaf| leaf.kind.to_string() == *kind)
        .filter_map(|leaf| {
            let (leaf_path, leaf_symbol) = leaf.base.location.split_once('#')?;
            if leaf_path != path {
                return None;
            }
            let (leaf_container, leaf_method) = leaf_symbol.rsplit_once("::")?;
            if leaf_container != container {
                return None;
            }
            let leaf_method_lower = leaf_method.to_ascii_lowercase();
            Some((
                string_affinity_rank(&requested_method_lower, &leaf_method_lower),
                levenshtein_distance(&requested_method_lower, &leaf_method_lower),
                leaf_method_lower
                    .len()
                    .abs_diff(requested_method_lower.len()),
                format!("symbol:{}:{}", leaf.base.location, leaf.kind),
            ))
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.3.cmp(&right.3))
    });
    candidates
        .into_iter()
        .map(|(_, _, _, selector)| selector)
        .take(DID_YOU_MEAN_LIMIT)
        .collect()
}

fn file_resolves(graph: &CodebaseGraphV1, path: &str) -> bool {
    graph.files.iter().any(|file| file.base.location == path)
}

fn containing_context_resolves(graph: &CodebaseGraphV1, path: &str, container: &str) -> bool {
    let normalized_type_name = normalized_container_type_name(container);
    graph.leaves.iter().any(|leaf| {
        let Some((leaf_path, leaf_symbol)) = leaf.base.location.split_once('#') else {
            return false;
        };
        if leaf_path != path {
            return false;
        }

        (matches!(leaf.kind, LeafKind::Impl) && leaf_symbol == container)
            || (is_type_like_kind(&leaf.kind) && leaf_symbol == normalized_type_name)
    })
}

fn normalized_container_type_name(container: &str) -> &str {
    let without_angles = container
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
        .unwrap_or(container);
    without_angles
        .split_once(" as ")
        .map(|(type_name, _)| type_name)
        .unwrap_or(without_angles)
}

fn is_type_like_kind(kind: &LeafKind) -> bool {
    matches!(
        kind,
        LeafKind::Class
            | LeafKind::SingletonClass
            | LeafKind::Enum
            | LeafKind::Struct
            | LeafKind::Record
            | LeafKind::Interface
            | LeafKind::Trait
            | LeafKind::Object
            | LeafKind::CompanionObject
    )
}

fn string_affinity_rank(needle: &str, candidate: &str) -> u8 {
    if candidate.starts_with(needle) {
        0
    } else if candidate.contains(needle) {
        1
    } else {
        2
    }
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    if left == right {
        return 0;
    }
    if left.is_empty() {
        return right.chars().count();
    }
    if right.is_empty() {
        return left.chars().count();
    }

    let right_chars = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut current = vec![0; right_chars.len() + 1];

    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let substitution = usize::from(left_char != *right_char);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right_chars.len()]
}

fn try_show_via_sql_index(
    context: &GraphCommandContext,
    selector: &Selector,
    depth: usize,
    max_siblings: usize,
    max_children: usize,
) -> Result<Option<ShowResult>, KnowledgeError> {
    if context.explicit_ref.is_none()
        && !context.explicit_knowledge_dir
        && let Some(workspace_root) = context.workspace_root.as_deref()
    {
        let _ = crate::pipeline::ensure_fresh(&context.knowledge_dir, workspace_root);
    }

    let read_target = match resolve_graph_read_target(
        context.workspace_root.as_deref(),
        context.explicit_ref.as_deref(),
    ) {
        Ok(target) => target,
        Err(_) => return Ok(None),
    };
    let graph_store = GraphObjectStore::new(context.knowledge_dir.join("graph"));
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
    let reader = match crate::graph::GraphIndexReader::open_current(
        graph_store.graph_sqlite_index_path(),
        &resolved.current_ref.root_graph_hash,
    ) {
        Ok(Some(reader)) => reader,
        Ok(None) | Err(_) => return Ok(None),
    };

    let Some(row) = reader
        .find_node_by_selector(&selector.to_string())
        .map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!("query graph sqlite index: {error}"))
        })?
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
        .map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!("read graph node: {error}"))
        })?;
    let lineage = reader
        .lineage_for(row.parent_id.as_deref(), depth)
        .map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!("query graph sqlite lineage: {error}"))
        })?;
    let siblings = if let Some(parent_id) = row.parent_id.as_deref() {
        reader
            .children_of(parent_id, max_siblings.saturating_add(1))
            .map_err(|error| {
                KnowledgeError::knowledge_unavailable(format!(
                    "query graph sqlite siblings: {error}"
                ))
            })?
            .into_iter()
            .filter(|sibling| sibling.id != row.id)
            .take(max_siblings)
            .collect()
    } else {
        Vec::new()
    };
    let children = reader.children_of(&row.id, max_children).map_err(|error| {
        KnowledgeError::knowledge_unavailable(format!("query graph sqlite children: {error}"))
    })?;

    Ok(Some(result_from_sql_node(
        &node, &lineage, &siblings, &children,
    )))
}

fn result_from_context(service: &GraphContextService<'_>, context: &NodeContext<'_>) -> ShowResult {
    let node = context.node;
    ShowResult {
        selector: service.selector_for_node(node),
        lineage: context
            .lineage
            .iter()
            .map(|node| service.selector_for_node(*node))
            .collect(),
        siblings: context
            .siblings
            .iter()
            .map(|node| service.selector_for_node(*node))
            .collect(),
        children: context
            .children
            .iter()
            .map(|node| service.selector_for_node(*node))
            .collect(),
        details: match node {
            crate::graph::navigator::GraphNodeRef::Leaf(leaf) => ShowNodeDetails::Leaf {
                source: leaf.source.clone(),
                start_line: leaf.start_line,
                end_line: leaf.end_line,
            },
            crate::graph::navigator::GraphNodeRef::File(file) => ShowNodeDetails::File {
                source: (!file.source.is_empty()).then(|| file.source.clone()),
                source_blob_hash: file.source_blob_hash.clone(),
                imports: file.imports.clone(),
                exports: file.exports.clone(),
                re_exports: file
                    .re_exports
                    .iter()
                    .map(|value| serde_json::to_value(value).unwrap_or(Value::Null))
                    .collect(),
            },
            crate::graph::navigator::GraphNodeRef::Dir(_) => ShowNodeDetails::Dir,
        },
    }
}

fn result_from_sql_node(
    node: &GraphNode,
    lineage: &[GraphIndexNodeRow],
    siblings: &[GraphIndexNodeRow],
    children: &[GraphIndexNodeRow],
) -> ShowResult {
    ShowResult {
        selector: selector_for_graph_node(node),
        lineage: lineage.iter().map(selector_for_index_row).collect(),
        siblings: siblings.iter().map(selector_for_index_row).collect(),
        children: children.iter().map(selector_for_index_row).collect(),
        details: match node {
            GraphNode::Leaf(leaf) => ShowNodeDetails::Leaf {
                source: leaf.source.clone(),
                start_line: leaf.start_line,
                end_line: leaf.end_line,
            },
            GraphNode::File(file) => ShowNodeDetails::File {
                source: (!file.source.is_empty()).then(|| file.source.clone()),
                source_blob_hash: file.source_blob_hash.clone(),
                imports: file.imports.clone(),
                exports: file.exports.clone(),
                re_exports: file
                    .re_exports
                    .iter()
                    .map(|value| serde_json::to_value(value).unwrap_or(Value::Null))
                    .collect(),
            },
            GraphNode::Dir(_) => ShowNodeDetails::Dir,
        },
    }
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

#[cfg(test)]
mod tests {
    use crate::graph::{BaseNodeFields, DirNode, FileNode, LeafNode};
    use crate::service::GraphContextService;

    use super::*;

    #[test]
    fn failed_method_on_resolvable_type_returns_did_you_mean() {
        let graph = graph_with_methods(vec![
            "load_layered",
            "default_for_data_root",
            "workflow_base_branch",
        ]);
        let selector: Selector = "symbol:src/runtime.rs#<RuntimeConfig>::load:method"
            .parse()
            .expect("valid selector");

        let error = resolution_error_for(&graph, &selector);

        assert_eq!(
            error.did_you_mean.first().map(String::as_str),
            Some("symbol:src/runtime.rs#<RuntimeConfig>::load_layered:method")
        );
    }

    #[test]
    fn failed_type_or_file_returns_no_suggestions() {
        let graph = graph_with_methods(vec!["load_layered"]);
        let missing_type: Selector = "symbol:src/runtime.rs#<MissingConfig>::load:method"
            .parse()
            .expect("valid selector");
        let missing_file: Selector = "symbol:src/missing.rs#<RuntimeConfig>::load:method"
            .parse()
            .expect("valid selector");

        assert!(
            resolution_error_for(&graph, &missing_type)
                .did_you_mean
                .is_empty()
        );
        assert!(
            resolution_error_for(&graph, &missing_file)
                .did_you_mean
                .is_empty()
        );
    }

    #[test]
    fn method_suggestions_are_bounded_by_cap() {
        let graph = graph_with_methods(vec![
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf",
        ]);
        let selector: Selector = "symbol:src/runtime.rs#<RuntimeConfig>::missing:method"
            .parse()
            .expect("valid selector");

        let error = resolution_error_for(&graph, &selector);

        assert_eq!(error.did_you_mean.len(), DID_YOU_MEAN_LIMIT);
    }

    fn resolution_error_for(graph: &CodebaseGraphV1, selector: &Selector) -> KnowledgeError {
        let service = GraphContextService::new(graph);
        let error = service
            .resolve_selector(selector)
            .expect_err("selector should be unresolved");
        invalid_selector_resolution_error(graph, selector, error)
    }

    fn graph_with_methods(method_names: Vec<&str>) -> CodebaseGraphV1 {
        let file_id = "file:src/runtime.rs";
        let mut leaf_children = vec!["symbol:src/runtime.rs#RuntimeConfig:struct".to_string()];
        let mut leaves = vec![leaf_node(
            "symbol:src/runtime.rs#RuntimeConfig:struct",
            "RuntimeConfig",
            "src/runtime.rs#RuntimeConfig",
            file_id,
            LeafKind::Struct,
        )];

        for method_name in method_names {
            let location = format!("src/runtime.rs#<RuntimeConfig>::{method_name}");
            let id = format!("symbol:{location}:method");
            leaf_children.push(id.clone());
            leaves.push(leaf_node(
                &id,
                method_name,
                &location,
                file_id,
                LeafKind::Method,
            ));
        }

        CodebaseGraphV1 {
            root_dir_id: "dir:.".to_string(),
            dirs: vec![DirNode {
                base: base_node("dir:.", ".", ".", None),
                dir_children: Vec::new(),
                file_children: vec![file_id.to_string()],
            }],
            files: vec![FileNode {
                base: base_node(file_id, "runtime.rs", "src/runtime.rs", Some("dir:.")),
                extension: Some("rs".to_string()),
                source_blob_hash: None,
                source: String::new(),
                imports: Vec::new(),
                exports: Vec::new(),
                re_exports: Vec::new(),
                leaf_children,
            }],
            leaves,
        }
    }

    fn leaf_node(
        id: &str,
        name: &str,
        location: &str,
        parent_id: &str,
        kind: LeafKind,
    ) -> LeafNode {
        LeafNode {
            base: base_node(id, name, location, Some(parent_id)),
            kind,
            source: String::new(),
            source_blob_hash: None,
            source_hash: None,
            file_hash_at_capture: None,
            history: Vec::new(),
            input_signature: Vec::new(),
            output_signature: Vec::new(),
            start_line: None,
            end_line: None,
            children: Vec::new(),
        }
    }

    fn base_node(id: &str, name: &str, location: &str, parent_id: Option<&str>) -> BaseNodeFields {
        BaseNodeFields {
            id: id.to_string(),
            identity_key: id.to_string(),
            object_hash: None,
            name: name.to_string(),
            location: location.to_string(),
            language: "rust".to_string(),
            description: String::new(),
            parent_id: parent_id.map(str::to_string),
            is_locked: false,
            lineage_locked: false,
            lock_owner: None,
            lock_reason: String::new(),
        }
    }
}
