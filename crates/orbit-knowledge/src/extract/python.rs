use tree_sitter::{Node, Parser};

use super::LanguageExtractor;
use super::common::{ExtractedLeaf, ExtractionResult, compute_source_hash};

pub struct PythonExtractor;

impl LanguageExtractor for PythonExtractor {
    fn language(&self) -> &str {
        "python"
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .expect("tree-sitter-python");

        let tree = match parser.parse(source, None) {
            Some(t) => t,
            None => return ExtractionResult { leaves: vec![] },
        };

        let mut leaves = Vec::new();
        extract_top_level(tree.root_node(), source, &mut leaves, None);
        ExtractionResult { leaves }
    }
}

fn get_name(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|n| n.utf8_text(source.as_bytes()).unwrap_or("").to_string())
}

fn node_source(node: Node, source: &str) -> String {
    source[node.start_byte()..node.end_byte()]
        .trim_end()
        .to_string()
}

fn extract_top_level(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent_class: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => extract_function(child, source, leaves, parent_class),
            "class_definition" => extract_class(child, source, leaves),
            "decorated_definition" => {
                if let Some(inner) = child.child_by_field_name("definition") {
                    match inner.kind() {
                        "function_definition" => {
                            extract_function(inner, source, leaves, parent_class)
                        }
                        "class_definition" => extract_class(inner, source, leaves),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_function(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent_class: Option<&str>,
) {
    let name = match get_name(node, source) {
        Some(n) => n,
        None => return,
    };

    let kind = if parent_class.is_some() {
        "method"
    } else {
        "function"
    };
    let src = node_source(node, source);

    leaves.push(ExtractedLeaf {
        qualified_name: name.clone(),
        name,
        kind: kind.to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: parent_class.map(str::to_string),
        children_qualified_names: vec![],
    });
}

fn extract_class(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let name = match get_name(node, source) {
        Some(n) => n,
        None => return,
    };

    let src = node_source(node, source);
    let mut children = Vec::new();

    // Extract methods from the class body
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "function_definition" => {
                    if let Some(method_name) = get_name(child, source) {
                        children.push(method_name.clone());
                    }
                    extract_function(child, source, leaves, Some(&name));
                }
                "decorated_definition" => {
                    if let Some(inner) = child.child_by_field_name("definition")
                        && inner.kind() == "function_definition"
                    {
                        if let Some(method_name) = get_name(inner, source) {
                            children.push(method_name.clone());
                        }
                        extract_function(inner, source, leaves, Some(&name));
                    }
                }
                _ => {}
            }
        }
    }

    leaves.push(ExtractedLeaf {
        qualified_name: name.clone(),
        name,
        kind: "class".to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: None,
        children_qualified_names: children,
    });
}
