//! Trait-implementor lookup.
//!
//! Iterates `LeafKind::Impl` leaves and re-parses each impl block with
//! tree-sitter-rust to recover the trait path and implementing type. This is
//! done on-demand because the stored `LeafNode` only carries the implementing
//! type name, not the trait being implemented.

use std::collections::HashSet;

use tree_sitter::{Node, Parser};

use crate::error::KnowledgeError;
use crate::graph::navigator::GraphNodeRef;
use crate::graph::nodes::{CodebaseGraphV1, LeafKind, LeafNode};
use orbit_graph_extract::Selector;

use super::GraphContextService;

/// Result record for a single trait implementation.
pub struct ImplementorHit {
    pub selector: String,
    pub file: String,
    /// The implementing type as written in source (`Foo`, `Foo<T>`, `T`, etc.).
    pub type_name: String,
    /// The full trait path as written (`Foo`, `crate::Foo<T>`), or `None` for
    /// inherent impls — those are filtered out of the result set in
    /// [`trait_implementors`].
    pub trait_path: Option<String>,
    /// True if the impl has `<T, ...>` generics with the implementing type
    /// being a generic parameter — a blanket impl like `impl<T> Foo for T`.
    pub is_blanket: bool,
}

/// Find all impl leaves whose `trait` path matches the trait named by
/// `trait_selector`.
///
/// Matching is by trailing identifier: a trait selector resolving to
/// `EngineHost` matches any impl whose trait path ends in `::EngineHost` or is
/// exactly `EngineHost`. This deliberately accepts `crate::engine::EngineHost`
/// and the bare form, since the graph does not carry enough type information
/// to disambiguate further.
pub fn trait_implementors<'a>(
    svc: &'a GraphContextService<'a>,
    graph: &'a CodebaseGraphV1,
    trait_selector: &Selector,
) -> Result<Vec<ImplementorHit>, KnowledgeError> {
    let trait_node = svc.resolve_selector(trait_selector)?;
    let trait_leaf = match trait_node {
        GraphNodeRef::Leaf(leaf) if matches!(leaf.kind, LeafKind::Trait) => leaf,
        _ => {
            return Err(KnowledgeError::invalid_data(format!(
                "`{trait_selector}` is not a trait leaf"
            )));
        }
    };
    let trait_name = &trait_leaf.base.name;

    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| KnowledgeError::invalid_data(format!("tree-sitter init: {e}")))?;

    // Unique leaf IDs make trait impls addressable as `<Type as Trait>`, but
    // older graph snapshots may still contain duplicate impl IDs. Dedupe on id
    // so a stale duplicate is only parsed once while those snapshots fall back.
    let mut seen_ids: HashSet<&str> = HashSet::new();
    let mut hits = Vec::new();
    for leaf in &graph.leaves {
        if !matches!(leaf.kind, LeafKind::Impl) {
            continue;
        }
        if leaf.base.language != "rust" {
            continue;
        }
        if !seen_ids.insert(leaf.base.id.as_str()) {
            continue;
        }
        let Some(parsed) = parse_impl(&mut parser, leaf) else {
            continue;
        };
        let Some(trait_path) = parsed.trait_path.as_deref() else {
            continue; // inherent impl, not a trait impl
        };
        if !trait_path_matches(trait_path, trait_name) {
            continue;
        }

        let file = leaf
            .base
            .location
            .split_once('#')
            .map(|(path, _)| path.to_string())
            .unwrap_or_else(|| leaf.base.location.clone());

        hits.push(ImplementorHit {
            selector: svc.selector_for_node(GraphNodeRef::Leaf(leaf)),
            file,
            type_name: parsed.type_name,
            trait_path: Some(trait_path.to_string()),
            is_blanket: parsed.is_blanket,
        });
    }

    Ok(hits)
}

struct ParsedImpl {
    type_name: String,
    trait_path: Option<String>,
    is_blanket: bool,
}

fn parse_impl(parser: &mut Parser, leaf: &LeafNode) -> Option<ParsedImpl> {
    let tree = parser.parse(&leaf.source, None)?;
    let root = tree.root_node();
    let impl_node = find_impl_node(root)?;

    let type_name_node = impl_node.child_by_field_name("type")?;
    let type_name = node_text(type_name_node, &leaf.source).to_string();
    let trait_node = impl_node.child_by_field_name("trait");
    let trait_path = trait_node.map(|n| node_text(n, &leaf.source).to_string());

    let type_params_text = impl_node
        .child_by_field_name("type_parameters")
        .map(|n| node_text(n, &leaf.source).to_string())
        .unwrap_or_default();
    let is_blanket = type_param_contains(&type_params_text, &type_name);

    Some(ParsedImpl {
        type_name,
        trait_path,
        is_blanket,
    })
}

/// Return true if `<...>` generic parameter list declares a type parameter
/// whose identifier exactly matches `type_name`. Used to detect blanket impls
/// like `impl<T> Foo for T`.
fn type_param_contains(type_params_text: &str, type_name: &str) -> bool {
    if type_params_text.is_empty() || type_name.is_empty() {
        return false;
    }
    let inner = type_params_text
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>');
    for part in inner.split(',') {
        let head: String = part
            .trim()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if head == type_name {
            return true;
        }
    }
    false
}

fn find_impl_node(root: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = root.walk();
    root.children(&mut cursor)
        .find(|child| child.kind() == "impl_item")
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    source
        .get(node.start_byte()..node.end_byte())
        .unwrap_or("")
        .trim()
}

/// Trailing-identifier match: `trait_path` matches `trait_name` when the path
/// ends in `::trait_name` (ignoring any trailing generic arguments) or equals
/// `trait_name` after stripping generics.
fn trait_path_matches(trait_path: &str, trait_name: &str) -> bool {
    let without_generics = trait_path
        .split('<')
        .next()
        .unwrap_or(trait_path)
        .trim_end_matches(':');
    let tail = without_generics
        .rsplit("::")
        .next()
        .unwrap_or(without_generics)
        .trim();
    tail == trait_name
}
