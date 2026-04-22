use tree_sitter::{Node, Parser};

use super::FileExtractor;
use super::common::{ExtractedLeaf, ExtractionResult, compute_source_hash};
use super::language::{FileKind, Language};

pub struct RustExtractor;

impl FileExtractor for RustExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Code(Language::Rust)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("tree-sitter-rust");

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
        .trim()
        .to_string()
}

fn extract_top_level(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => extract_function(child, source, leaves, None),
            "struct_item" | "enum_item" => extract_struct_like(child, source, leaves),
            "trait_item" => extract_trait(child, source, leaves),
            "impl_item" => extract_impl(child, source, leaves),
            "mod_item" => extract_mod(child, source, leaves),
            "type_item" => extract_type_alias(child, source, leaves),
            "const_item" | "static_item" => extract_const_static(child, source, leaves),
            _ => {}
        }
    }
}

fn extract_function(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    parent: Option<&str>,
) {
    let name = match get_name(node, source) {
        Some(n) => n,
        None => return,
    };

    let src = node_source(node, source);
    let kind = if parent.is_some() {
        "method"
    } else {
        "function"
    };
    let qualified = match parent {
        Some(p) => format!("{p}::{name}"),
        None => name.clone(),
    };

    leaves.push(ExtractedLeaf {
        qualified_name: qualified,
        name,
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

fn extract_struct_like(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let name = match get_name(node, source) {
        Some(n) => n,
        None => return,
    };
    let src = node_source(node, source);

    leaves.push(ExtractedLeaf {
        qualified_name: name.clone(),
        name,
        kind: "struct".to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: None,
        children_qualified_names: vec![],
        depth: None,
    });
}

fn extract_trait(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let name = match get_name(node, source) {
        Some(n) => n,
        None => return,
    };
    let src = node_source(node, source);

    leaves.push(ExtractedLeaf {
        qualified_name: name.clone(),
        name,
        kind: "trait".to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: None,
        children_qualified_names: vec![],
        depth: None,
    });
}

fn extract_impl(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    // For impl blocks, the "type" field has the implementing type name
    let name = match node.child_by_field_name("type") {
        Some(n) => n.utf8_text(source.as_bytes()).unwrap_or("").to_string(),
        None => return,
    };

    if name.is_empty() {
        return;
    }

    let src = node_source(node, source);
    let mut children = Vec::new();

    // Extract methods from the body (declaration_list)
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "function_item" {
                extract_function(child, source, leaves, Some(&name));
                if let Some(method_name) = get_name(child, source) {
                    children.push(format!("{name}::{method_name}"));
                }
            }
        }
    }

    leaves.push(ExtractedLeaf {
        qualified_name: name.clone(),
        name,
        kind: "impl".to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: None,
        children_qualified_names: children,
        depth: None,
    });
}

fn extract_mod(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let name = match get_name(node, source) {
        Some(n) => n,
        None => return,
    };
    let src = node_source(node, source);

    leaves.push(ExtractedLeaf {
        qualified_name: name.clone(),
        name,
        kind: "module".to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: None,
        children_qualified_names: vec![],
        depth: None,
    });
}

fn extract_type_alias(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let name = match get_name(node, source) {
        Some(n) => n,
        None => return,
    };
    let src = node_source(node, source);

    leaves.push(ExtractedLeaf {
        qualified_name: name.clone(),
        name,
        kind: "struct".to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: None,
        children_qualified_names: vec![],
        depth: None,
    });
}

fn extract_const_static(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let name = match get_name(node, source) {
        Some(n) => n,
        None => return,
    };
    let src = node_source(node, source);

    leaves.push(ExtractedLeaf {
        qualified_name: name.clone(),
        name,
        kind: "field".to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: None,
        children_qualified_names: vec![],
        depth: None,
    });
}
