// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use tree_sitter::{Node, Parser};

use super::FileExtractor;
use super::common::{
    ExtractedLeaf, ExtractionResult, compute_source_hash, finalize_unique_qualified_names,
};
use super::language::{FileKind, Language};

pub struct CExtractor;

impl FileExtractor for CExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Code(Language::C)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .expect("tree-sitter-c");

        let tree = match parser.parse(source, None) {
            Some(t) => t,
            None => return ExtractionResult::default(),
        };

        let mut leaves = Vec::new();
        extract_top_level(tree.root_node(), source, &mut leaves);
        finalize_unique_qualified_names(&mut leaves);
        ExtractionResult {
            leaves,
            ..Default::default()
        }
    }
}

fn get_name(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|n| node_text(n, source))
        .filter(|name| !name.is_empty())
}

fn node_text(node: Node, source: &str) -> String {
    node.utf8_text(source.as_bytes())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn node_source(node: Node, source: &str) -> String {
    source[node.start_byte()..node.end_byte()]
        .trim_end()
        .to_string()
}

fn extract_top_level(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }

        match child.kind() {
            "function_definition" => extract_function_definition(child, source, leaves),
            "declaration" => extract_declaration(child, source, leaves),
            "type_definition" => extract_type_definition(child, source, leaves),
            "struct_specifier" | "union_specifier" | "enum_specifier" => {
                extract_tag_specifier(child, source, leaves)
            }
            "preproc_def" | "preproc_function_def" => extract_macro(child, source, leaves),
            "preproc_if" | "preproc_ifdef" | "preproc_elif" | "preproc_elifdef"
            | "preproc_else" => extract_top_level(child, source, leaves),
            _ => {}
        }
    }
}

fn extract_function_definition(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let Some(declarator) = node.child_by_field_name("declarator") else {
        return;
    };
    let Some(name) = declarator_name(declarator, source) else {
        return;
    };

    push_leaf(node, source, leaves, &name, &name, "function");
}

fn extract_declaration(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    if let Some(type_node) = node.child_by_field_name("type") {
        extract_tag_specifier(type_node, source, leaves);
    }

    for declarator in declarators(node) {
        let Some(name) = declarator_name(declarator, source) else {
            continue;
        };

        if is_function_prototype_declarator(declarator, source) {
            // Prototypes and definitions both represent callable C symbols.
            // The kind tag distinguishes header/source declarations from
            // definitions because ExtractedLeaf has no separate signature field.
            push_leaf(node, source, leaves, &name, &name, "function_declaration");
        } else {
            push_leaf(node, source, leaves, &name, &name, "global");
        }
    }
}

fn extract_type_definition(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    if let Some(type_node) = node.child_by_field_name("type") {
        extract_tag_specifier(type_node, source, leaves);
    }

    for declarator in declarators(node) {
        let Some(name) = declarator_name(declarator, source) else {
            continue;
        };
        push_leaf(node, source, leaves, &name, &name, "type_alias");
    }
}

fn extract_tag_specifier(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let kind = match node.kind() {
        "struct_specifier" => "struct",
        "union_specifier" => "union",
        "enum_specifier" => "enum",
        _ => return,
    };

    if node.child_by_field_name("body").is_none() {
        return;
    }

    let Some(name) = get_name(node, source) else {
        return;
    };
    push_leaf(node, source, leaves, &name, &name, kind);
}

fn extract_macro(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    push_leaf(node, source, leaves, &name, &name, "macro");
}

fn declarators(node: Node) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if is_declarator_kind(child.kind()) {
            nodes.push(child);
        }
    }
    nodes
}

fn declarator_name(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" | "field_identifier" => Some(node_text(node, source)),
        "init_declarator" | "function_declarator" | "array_declarator" | "pointer_declarator" => {
            child_declarator(node).and_then(|child| declarator_name(child, source))
        }
        "attributed_declarator" | "parenthesized_declarator" => {
            first_declarator_child(node).and_then(|child| declarator_name(child, source))
        }
        _ => None,
    }
    .filter(|name| !name.is_empty())
}

fn is_function_prototype_declarator(node: Node, source: &str) -> bool {
    if node.kind() != "function_declarator" {
        return false;
    }

    let Some(inner) = child_declarator(node) else {
        return false;
    };
    declarator_name(inner, source).is_some() && !contains_parenthesized_pointer(inner)
}

fn contains_parenthesized_pointer(node: Node) -> bool {
    if node.kind() == "parenthesized_declarator"
        && first_declarator_child(node)
            .map(|child| child.kind() == "pointer_declarator")
            .unwrap_or(false)
    {
        return true;
    }

    child_declarator(node)
        .map(contains_parenthesized_pointer)
        .unwrap_or(false)
}

fn child_declarator(node: Node) -> Option<Node> {
    node.child_by_field_name("declarator")
        .or_else(|| first_declarator_child(node))
}

fn first_declarator_child(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| is_declarator_kind(child.kind()))
}

fn is_declarator_kind(kind: &str) -> bool {
    matches!(
        kind,
        "array_declarator"
            | "attributed_declarator"
            | "function_declarator"
            | "identifier"
            | "init_declarator"
            | "parenthesized_declarator"
            | "pointer_declarator"
            | "type_identifier"
    )
}

fn push_leaf(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    name: &str,
    qualified_name: &str,
    kind: &str,
) {
    let src = node_source(node, source);
    let start_line = node.start_position().row + 1;
    let line_count = src.lines().count().max(1);
    leaves.push(ExtractedLeaf {
        qualified_name: qualified_name.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
        start_line,
        end_line: start_line + line_count - 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: None,
        children_qualified_names: vec![],
        depth: None,
    });
}
