use tree_sitter::{Node, Parser};

use super::FileExtractor;
use super::common::{
    ExtractedLeaf, ExtractionResult, compute_source_hash, finalize_unique_qualified_names,
};
use super::language::{FileKind, Language};

pub struct PythonExtractor;

impl FileExtractor for PythonExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Code(Language::Python)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .expect("tree-sitter-python");

        let tree = match parser.parse(source, None) {
            Some(t) => t,
            None => return ExtractionResult::default(),
        };

        let mut leaves = Vec::new();
        extract_top_level(tree.root_node(), source, &mut leaves, None);
        finalize_unique_qualified_names(&mut leaves);
        ExtractionResult {
            leaves,
            ..Default::default()
        }
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
            "function_definition" => {
                extract_function(child, source, leaves, parent_class);
            }
            "class_definition" => {
                extract_class(child, source, leaves, parent_class);
            }
            "decorated_definition" => {
                if let Some(inner) = child.child_by_field_name("definition") {
                    match inner.kind() {
                        "function_definition" => {
                            extract_function(inner, source, leaves, parent_class);
                        }
                        "class_definition" => {
                            extract_class(inner, source, leaves, parent_class);
                        }
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
) -> Option<String> {
    let name = get_name(node, source)?;

    let kind = if parent_class.is_some() {
        "method"
    } else {
        "function"
    };
    let src = node_source(node, source);
    let qualified_name = qualify_name(parent_class, &name);

    leaves.push(ExtractedLeaf {
        qualified_name: qualified_name.clone(),
        name,
        kind: kind.to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: parent_class.map(str::to_string),
        children_qualified_names: vec![],
        depth: None,
    });
    Some(qualified_name)
}

fn extract_class(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent_class: Option<&str>,
) -> Option<String> {
    let name = get_name(node, source)?;

    let src = node_source(node, source);
    let qualified_name = qualify_name(parent_class, &name);
    let mut children = Vec::new();

    // Extract methods from the class body
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "function_definition" => {
                    if let Some(method_name) =
                        extract_function(child, source, leaves, Some(&qualified_name))
                    {
                        children.push(method_name);
                    }
                }
                "class_definition" => {
                    extract_class(child, source, leaves, Some(&qualified_name));
                }
                "decorated_definition" => {
                    if let Some(inner) = child.child_by_field_name("definition") {
                        match inner.kind() {
                            "function_definition" => {
                                if let Some(method_name) =
                                    extract_function(inner, source, leaves, Some(&qualified_name))
                                {
                                    children.push(method_name);
                                }
                            }
                            "class_definition" => {
                                extract_class(inner, source, leaves, Some(&qualified_name));
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    leaves.push(ExtractedLeaf {
        qualified_name: qualified_name.clone(),
        name,
        kind: "class".to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: parent_class.map(str::to_string),
        children_qualified_names: children,
        depth: None,
    });
    Some(qualified_name)
}

fn qualify_name(parent: Option<&str>, name: &str) -> String {
    match parent {
        Some(parent) => format!("{parent}.{name}"),
        None => name.to_string(),
    }
}
