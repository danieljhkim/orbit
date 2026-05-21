// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use tree_sitter::{Node, Parser};

use super::FileExtractor;
use super::common::{
    ExtractedLeaf, ExtractionResult, compute_source_hash, finalize_unique_qualified_names,
};
use super::language::{FileKind, Language};

pub struct RubyExtractor;

impl FileExtractor for RubyExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Code(Language::Ruby)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_ruby::LANGUAGE.into())
            .expect("tree-sitter-ruby");

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
        Some(parent) if !name.contains("::") => format!("{parent}::{name}"),
        _ => name.to_string(),
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

        match child.kind() {
            "module" => {
                if let Some(qualified_name) = extract_scope(child, source, leaves, parent, "module")
                {
                    children.push(qualified_name);
                }
            }
            "class" => {
                if let Some(qualified_name) = extract_scope(child, source, leaves, parent, "class")
                {
                    children.push(qualified_name);
                }
            }
            "singleton_class" => {
                if let Some(qualified_name) = extract_singleton_class(child, source, leaves, parent)
                {
                    children.push(qualified_name);
                }
            }
            "method" => {
                if let Some(qualified_name) = extract_method(child, source, leaves, parent) {
                    children.push(qualified_name);
                }
            }
            "singleton_method" => {
                if let Some(qualified_name) =
                    extract_singleton_method(child, source, leaves, parent)
                {
                    children.push(qualified_name);
                }
            }
            "assignment" if parent.is_none() => {
                if let Some(qualified_name) = extract_top_level_constant(child, source, leaves) {
                    children.push(qualified_name);
                }
            }
            "call" => {
                children.extend(extract_attr_call(child, source, leaves, parent));
            }
            "body_statement" | "program" => {
                children.extend(extract_children(child, source, leaves, parent));
            }
            _ => {}
        }
    }

    children
}

fn extract_scope(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
    kind: &str,
) -> Option<String> {
    let name = get_name(node, source)?;
    let qualified_name = qualify_name(parent, &name);
    let children = node
        .child_by_field_name("body")
        .map(|body| extract_children(body, source, leaves, Some(&qualified_name)))
        .unwrap_or_default();
    let src = node_source(node, source);

    leaves.push(ExtractedLeaf {
        qualified_name: qualified_name.clone(),
        name,
        kind: kind.to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: parent.map(str::to_string),
        children_qualified_names: children,
        depth: None,
    });

    Some(qualified_name)
}

fn extract_singleton_class(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
) -> Option<String> {
    let value = node.child_by_field_name("value")?;
    let name = node_text(value, source);
    if name.is_empty() {
        return None;
    }

    let qualified_name = match parent {
        Some(parent) => format!("{parent}::{name}"),
        None => name.clone(),
    };
    let children = node
        .child_by_field_name("body")
        .map(|body| extract_children(body, source, leaves, Some(&qualified_name)))
        .unwrap_or_default();
    let src = node_source(node, source);

    leaves.push(ExtractedLeaf {
        qualified_name: qualified_name.clone(),
        name,
        kind: "singleton_class".to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: parent.map(str::to_string),
        children_qualified_names: children,
        depth: None,
    });

    Some(qualified_name)
}

fn extract_method(
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
        &qualified_name,
        &name,
        "method",
        parent,
    );
    Some(qualified_name)
}

fn extract_singleton_method(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
) -> Option<String> {
    let object = node.child_by_field_name("object")?;
    let method_name = get_name(node, source)?;
    let object_name = node_text(object, source);
    if object_name.is_empty() {
        return None;
    }

    let name = format!("{object_name}.{method_name}");
    let qualified_name = match parent {
        Some(parent) if object_name == "self" => format!("{parent}.{method_name}"),
        _ => name.clone(),
    };

    push_leaf(
        node,
        source,
        leaves,
        &qualified_name,
        &name,
        "singleton_method",
        parent,
    );
    Some(qualified_name)
}

fn extract_top_level_constant(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
) -> Option<String> {
    let left = node.child_by_field_name("left")?;
    if left.kind() != "constant" {
        return None;
    }

    let name = node_text(left, source);
    if name.is_empty() {
        return None;
    }

    push_leaf(node, source, leaves, &name, &name, "constant", None);
    Some(name)
}

// Ruby attr_* declarations define methods at runtime, so the graph records
// each generated accessor as a method leaf while keeping the declaration's span.
fn extract_attr_call(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
) -> Vec<String> {
    if node.child_by_field_name("receiver").is_some() {
        return Vec::new();
    }

    let Some(method) = node.child_by_field_name("method") else {
        return Vec::new();
    };
    let method_name = node_text(method, source);
    if !matches!(
        method_name.as_str(),
        "attr_accessor" | "attr_reader" | "attr_writer"
    ) {
        return Vec::new();
    }

    let Some(arguments) = node.child_by_field_name("arguments") else {
        return Vec::new();
    };

    let accessors = attr_accessor_names(&method_name, arguments, source);
    let mut children = Vec::new();
    for name in accessors {
        let qualified_name = qualify_name(parent, &name);
        push_leaf(
            node,
            source,
            leaves,
            &qualified_name,
            &name,
            "method",
            parent,
        );
        children.push(qualified_name);
    }

    children
}

fn attr_accessor_names(method_name: &str, arguments: Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = arguments.walk();

    for child in arguments.children(&mut cursor) {
        if !matches!(child.kind(), "simple_symbol" | "string") {
            continue;
        }
        let Some(name) = literal_attribute_name(child, source) else {
            continue;
        };

        match method_name {
            "attr_accessor" => {
                names.push(name.clone());
                names.push(format!("{name}="));
            }
            "attr_reader" => names.push(name),
            "attr_writer" => names.push(format!("{name}=")),
            _ => {}
        }
    }

    names
}

fn literal_attribute_name(node: Node, source: &str) -> Option<String> {
    let raw = node_text(node, source);
    let name = raw
        .trim_start_matches(':')
        .trim_matches('"')
        .trim_matches('\'')
        .to_string();
    (!name.is_empty()).then_some(name)
}

fn push_leaf(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    qualified_name: &str,
    name: &str,
    kind: &str,
    parent: Option<&str>,
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
        children_qualified_names: vec![],
        depth: None,
    });
}

#[cfg(test)]
mod tests;
