// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use tree_sitter::{Node, Parser};

use super::FileExtractor;
use super::common::{
    ExtractedLeaf, ExtractionResult, compute_source_hash, finalize_unique_qualified_names,
};
use super::language::{FileKind, Language};

pub struct CSharpExtractor;

impl FileExtractor for CSharpExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Code(Language::CSharp)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
            .expect("tree-sitter-c-sharp");

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
    let mut scoped_parent = parent.map(str::to_string);
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }

        if child.kind() == "file_scoped_namespace_declaration" {
            if let Some(qualified_name) =
                extract_file_scoped_namespace(child, source, leaves, parent)
            {
                scoped_parent = Some(qualified_name.clone());
                children.push(qualified_name);
            }
            continue;
        }

        children.extend(extract_node(
            child,
            source,
            leaves,
            scoped_parent.as_deref().or(parent),
        ));
    }

    children
}

fn extract_node(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
) -> Vec<String> {
    match node.kind() {
        "namespace_declaration" => extract_namespace(node, source, leaves, parent),
        "class_declaration" => extract_type(node, source, leaves, parent, "class"),
        "struct_declaration" => extract_type(node, source, leaves, parent, "struct"),
        "record_declaration" => extract_type(node, source, leaves, parent, "record"),
        "interface_declaration" => extract_type(node, source, leaves, parent, "interface"),
        "enum_declaration" => extract_type(node, source, leaves, parent, "enum"),
        "method_declaration" | "constructor_declaration" => {
            extract_named_member(node, source, leaves, parent, "method")
        }
        "property_declaration" => extract_named_member(node, source, leaves, parent, "property"),
        "field_declaration" => extract_variable_members(node, source, leaves, parent, "field"),
        "event_declaration" => extract_named_member(node, source, leaves, parent, "event"),
        "event_field_declaration" => {
            extract_variable_members(node, source, leaves, parent, "event")
        }
        "delegate_declaration" => extract_named_member(node, source, leaves, parent, "delegate"),
        "compilation_unit" | "declaration_list" | "declaration" | "type_declaration"
        | "preproc_if" | "preproc_elif" | "preproc_else" => {
            extract_children(node, source, leaves, parent)
        }
        _ => Vec::new(),
    }
}

fn extract_namespace(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
) -> Vec<String> {
    let Some(name) = get_name(node, source) else {
        return Vec::new();
    };
    let qualified_name = qualify_name(parent, &name);
    let children = node
        .child_by_field_name("body")
        .map(|body| extract_children(body, source, leaves, Some(&qualified_name)))
        .unwrap_or_default();

    push_leaf(
        node,
        source,
        leaves,
        &name,
        &qualified_name,
        "namespace",
        parent,
        children,
    );

    vec![qualified_name]
}

fn extract_file_scoped_namespace(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
) -> Option<String> {
    let name = get_name(node, source)?;
    let qualified_name = qualify_name(parent, &name);
    push_leaf(
        node,
        source,
        leaves,
        &name,
        &qualified_name,
        "namespace",
        parent,
        Vec::new(),
    );
    Some(qualified_name)
}

fn extract_type(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
    kind: &str,
) -> Vec<String> {
    let Some(name) = get_name(node, source) else {
        return Vec::new();
    };
    let qualified_name = qualify_name(parent, &name);
    let children = node
        .child_by_field_name("body")
        .map(|body| extract_children(body, source, leaves, Some(&qualified_name)))
        .unwrap_or_default();

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

    vec![qualified_name]
}

fn extract_named_member(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
    kind: &str,
) -> Vec<String> {
    let Some(name) = get_name(node, source) else {
        return Vec::new();
    };
    let qualified_name = qualify_name(parent, &name);
    push_leaf(
        node,
        source,
        leaves,
        &name,
        &qualified_name,
        kind,
        parent,
        Vec::new(),
    );
    vec![qualified_name]
}

fn extract_variable_members(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
    kind: &str,
) -> Vec<String> {
    let mut children = Vec::new();
    for name in variable_names(node, source) {
        let qualified_name = qualify_name(parent, &name);
        push_leaf(
            node,
            source,
            leaves,
            &name,
            &qualified_name,
            kind,
            parent,
            Vec::new(),
        );
        children.push(qualified_name);
    }
    children
}

fn variable_names(node: Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        if child.kind() != "variable_declaration" {
            continue;
        }

        let mut declaration_cursor = child.walk();
        for declarator in child.named_children(&mut declaration_cursor) {
            if declarator.kind() != "variable_declarator" {
                continue;
            }
            if let Some(name) = get_name(declarator, source) {
                names.push(name);
            }
        }
    }

    names
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
