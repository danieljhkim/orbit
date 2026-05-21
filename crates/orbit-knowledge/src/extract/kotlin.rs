// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use tree_sitter::{Node, Parser};

use super::FileExtractor;
use super::common::{
    ExtractedLeaf, ExtractionResult, compute_source_hash, finalize_unique_qualified_names,
};
use super::language::{FileKind, Language};

pub struct KotlinExtractor;

impl FileExtractor for KotlinExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Code(Language::Kotlin)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
            .expect("tree-sitter-kotlin-ng");

        let tree = match parser.parse(source, None) {
            Some(t) => t,
            None => return ExtractionResult::default(),
        };

        let mut leaves = Vec::new();
        extract_children(tree.root_node(), source, &mut leaves, None);
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

fn qualify_name(parent: Option<&str>, name: &str) -> String {
    match parent {
        Some(parent) => format!("{parent}::{name}"),
        None => name.to_string(),
    }
}

fn extract_children(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
) -> Vec<String> {
    let mut children = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }

        if let Some(qualified_name) = extract_node(child, source, leaves, parent) {
            children.push(qualified_name);
        }
    }

    children
}

fn extract_node(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
) -> Option<String> {
    match node.kind() {
        "package_header" => {
            extract_package(node, source, leaves);
            None
        }
        "statement" | "declaration" | "class_member_declaration" => {
            extract_children(node, source, leaves, parent);
            None
        }
        "class_declaration" => {
            extract_class(node, source, leaves, parent);
            None
        }
        "object_declaration" => {
            extract_object(node, source, leaves, parent);
            None
        }
        "companion_object" => {
            extract_companion_object(node, source, leaves, parent);
            None
        }
        "function_declaration" => extract_function(node, source, leaves, parent),
        "property_declaration" => {
            extract_property(node, source, leaves, parent);
            None
        }
        "type_alias" => {
            extract_type_alias(node, source, leaves, parent);
            None
        }
        _ => None,
    }
}

fn extract_package(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let mut cursor = node.walk();
    let Some(name_node) = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == "qualified_identifier")
    else {
        return;
    };
    let name = node_text(name_node, source);
    if name.is_empty() {
        return;
    }

    push_leaf(
        node,
        source,
        leaves,
        &name,
        &name,
        "package",
        None,
        Vec::new(),
    );
}

fn extract_class(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>, parent: Option<&str>) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let qualified_name = qualify_name(parent, &name);
    let children = extract_type_children(node, source, leaves, &qualified_name);
    let kind = if is_interface(node, source) {
        "interface"
    } else {
        "class"
    };

    push_leaf(
        node,
        source,
        leaves,
        &name,
        &qualified_name,
        kind,
        parent,
        children,
    );
}

fn extract_object(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>, parent: Option<&str>) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    let qualified_name = qualify_name(parent, &name);
    let children = extract_type_children(node, source, leaves, &qualified_name);

    push_leaf(
        node,
        source,
        leaves,
        &name,
        &qualified_name,
        "object",
        parent,
        children,
    );
}

fn extract_companion_object(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
) {
    let name = get_name(node, source).unwrap_or_else(|| "Companion".to_string());
    let qualified_name = qualify_name(parent, &name);
    let children = extract_type_children(node, source, leaves, &qualified_name);

    push_leaf(
        node,
        source,
        leaves,
        &name,
        &qualified_name,
        "companion_object",
        parent,
        children,
    );
}

fn extract_type_children(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: &str,
) -> Vec<String> {
    let mut children = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if !matches!(child.kind(), "class_body" | "enum_class_body") {
            continue;
        }
        children.extend(extract_children(child, source, leaves, Some(parent)));
    }

    children
}

fn extract_function(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
) -> Option<String> {
    let name = get_name(node, source)?;
    let (display_name, qualified_name, kind, leaf_parent) = match parent {
        Some(parent) => {
            let qualified_name = format!("{parent}::{name}");
            (
                name,
                qualified_name.clone(),
                "method",
                Some(parent.to_string()),
            )
        }
        None => {
            // Kotlin extension functions stay standalone function leaves. The
            // receiver is recorded as `Receiver.function` in both `name` and
            // `qualified_name`, while the full source keeps the original
            // Kotlin signature.
            let display_name = match receiver_type(node, source) {
                Some(receiver) => format!("{receiver}.{name}"),
                None => name,
            };
            (display_name.clone(), display_name, "function", None)
        }
    };

    push_leaf(
        node,
        source,
        leaves,
        &display_name,
        &qualified_name,
        kind,
        leaf_parent.as_deref(),
        Vec::new(),
    );

    parent.map(|_| qualified_name)
}

fn extract_property(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
) {
    for name in property_names(node, source) {
        let qualified_name = qualify_name(parent, &name);
        push_leaf(
            node,
            source,
            leaves,
            &name,
            &qualified_name,
            "field",
            parent,
            Vec::new(),
        );
    }
}

fn extract_type_alias(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
) {
    let Some(name_node) = node.child_by_field_name("type") else {
        return;
    };
    let name = node_text(name_node, source);
    if name.is_empty() {
        return;
    }

    let qualified_name = qualify_name(parent, &name);
    push_leaf(
        node,
        source,
        leaves,
        &name,
        &qualified_name,
        "type_alias",
        parent,
        Vec::new(),
    );
}

fn is_interface(node: Node, source: &str) -> bool {
    let Some(name_node) = node.child_by_field_name("name") else {
        return false;
    };
    source[node.start_byte()..name_node.start_byte()]
        .split_whitespace()
        .any(|part| part == "interface")
}

fn receiver_type(node: Node, source: &str) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    let mut cursor = node.walk();
    let receiver = node
        .named_children(&mut cursor)
        .take_while(|child| child.end_byte() <= name_node.start_byte())
        .filter(|child| is_type_node(child.kind()))
        .last()?;
    let receiver = node_text(receiver, source);
    (!receiver.is_empty()).then_some(receiver)
}

fn is_type_node(kind: &str) -> bool {
    matches!(
        kind,
        "type"
            | "user_type"
            | "nullable_type"
            | "non_nullable_type"
            | "parenthesized_type"
            | "function_type"
            | "dynamic"
    )
}

fn property_names(node: Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "variable_declaration" => collect_variable_names(child, source, &mut names),
            "multi_variable_declaration" => collect_variable_names(child, source, &mut names),
            _ => {}
        }
    }
    names
}

fn collect_variable_names(node: Node, source: &str, names: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = node_text(child, source);
                if !name.is_empty() {
                    names.push(name);
                }
            }
            "variable_declaration" => collect_variable_names(child, source, names),
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_leaf(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    name: &str,
    qualified_name: &str,
    kind: &str,
    parent: Option<&str>,
    children: Vec<String>,
) {
    let src = node_source(node, source);
    leaves.push(ExtractedLeaf {
        qualified_name: qualified_name.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: parent.map(str::to_string),
        children_qualified_names: children,
        depth: None,
    });
}

#[cfg(test)]
mod tests;
