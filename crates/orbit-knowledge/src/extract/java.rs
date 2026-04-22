use tree_sitter::{Node, Parser};

use super::FileExtractor;
use super::common::{ExtractedLeaf, ExtractionResult, compute_source_hash};
use super::language::{FileKind, Language};

pub struct JavaExtractor;

impl FileExtractor for JavaExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Code(Language::Java)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .expect("tree-sitter-java");

        let tree = match parser.parse(source, None) {
            Some(t) => t,
            None => return ExtractionResult { leaves: vec![] },
        };

        let mut leaves = Vec::new();
        extract_top_level(tree.root_node(), source, &mut leaves);
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

fn extract_top_level(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" => extract_type(child, source, leaves, "class"),
            "interface_declaration" => extract_type(child, source, leaves, "interface"),
            "enum_declaration" | "record_declaration" => {
                extract_type(child, source, leaves, "class")
            }
            _ => {}
        }
    }
}

fn extract_type(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>, kind: &str) {
    let name = match get_name(node, source) {
        Some(name) => name,
        None => return,
    };

    let mut children = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "method_declaration" => {
                    if let Some(qualified_name) = extract_method(child, source, leaves, &name) {
                        children.push(qualified_name);
                    }
                }
                "constructor_declaration" => {
                    if let Some(qualified_name) = extract_constructor(child, source, leaves, &name)
                    {
                        children.push(qualified_name);
                    }
                }
                _ => {}
            }
        }
    }

    let src = node_source(node, source);
    leaves.push(ExtractedLeaf {
        qualified_name: name.clone(),
        name,
        kind: kind.to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: None,
        children_qualified_names: children,
        depth: None,
    });
}

fn extract_method(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: &str,
) -> Option<String> {
    let name = get_name(node, source)?;
    let qualified_name = format!("{parent}::{name}");
    let src = node_source(node, source);

    leaves.push(ExtractedLeaf {
        qualified_name: qualified_name.clone(),
        name,
        kind: "method".to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: Some(parent.to_string()),
        children_qualified_names: vec![],
        depth: None,
    });

    Some(qualified_name)
}

fn extract_constructor(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: &str,
) -> Option<String> {
    let name = get_name(node, source)?;
    let qualified_name = format!("{parent}::{name}");
    let src = node_source(node, source);

    leaves.push(ExtractedLeaf {
        qualified_name: qualified_name.clone(),
        name,
        kind: "method".to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: Some(parent.to_string()),
        children_qualified_names: vec![],
        depth: None,
    });

    Some(qualified_name)
}
