use tree_sitter::{Node, Parser};

use super::LanguageExtractor;
use super::common::{ExtractedLeaf, ExtractionResult, compute_source_hash};

pub struct JavaScriptExtractor;

impl LanguageExtractor for JavaScriptExtractor {
    fn language(&self) -> &str {
        "javascript"
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .expect("tree-sitter-javascript");

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
        let declaration = unwrap_declaration(child);
        match declaration.kind() {
            "function_declaration" => extract_function(declaration, source, leaves),
            "class_declaration" => extract_class(declaration, source, leaves),
            "lexical_declaration" | "variable_declaration" => {
                extract_binding_declaration(declaration, source, leaves)
            }
            _ => {}
        }
    }
}

fn unwrap_declaration(node: Node) -> Node {
    if node.kind() == "export_statement" {
        node.child_by_field_name("declaration").unwrap_or(node)
    } else {
        node
    }
}

fn extract_function(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let name = match get_name(node, source) {
        Some(name) => name,
        None => return,
    };

    let src = node_source(node, source);
    leaves.push(ExtractedLeaf {
        qualified_name: name.clone(),
        name,
        kind: "function".to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: None,
        children_qualified_names: vec![],
    });
}

fn extract_class(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let name = match get_name(node, source) {
        Some(name) => name,
        None => return,
    };

    let mut children = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() != "method_definition" {
                continue;
            }

            if let Some(qualified_name) = extract_method(child, source, leaves, &name) {
                children.push(qualified_name);
            }
        }
    }

    let src = node_source(node, source);
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
    });

    Some(qualified_name)
}

fn extract_binding_declaration(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "variable_declarator" {
            continue;
        }

        let name = match get_name(child, source) {
            Some(name) => name,
            None => continue,
        };

        let Some(value) = child.child_by_field_name("value") else {
            continue;
        };

        match value.kind() {
            "arrow_function" | "function_expression" => {
                let src = node_source(child, source);
                leaves.push(ExtractedLeaf {
                    qualified_name: name.clone(),
                    name,
                    kind: "function".to_string(),
                    start_line: child.start_position().row + 1,
                    end_line: child.end_position().row + 1,
                    source: src.clone(),
                    source_hash: compute_source_hash(&src),
                    parent_qualified_name: None,
                    children_qualified_names: vec![],
                });
            }
            "class" => extract_class_binding(child, value, source, leaves, &name),
            _ => {}
        }
    }
}

fn extract_class_binding(
    declarator: Node,
    class_expr: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    binding_name: &str,
) {
    let mut children = Vec::new();
    if let Some(body) = class_expr.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() != "method_definition" {
                continue;
            }

            if let Some(qualified_name) = extract_method(child, source, leaves, binding_name) {
                children.push(qualified_name);
            }
        }
    }

    let src = node_source(declarator, source);
    leaves.push(ExtractedLeaf {
        qualified_name: binding_name.to_string(),
        name: binding_name.to_string(),
        kind: "class".to_string(),
        start_line: declarator.start_position().row + 1,
        end_line: declarator.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: None,
        children_qualified_names: children,
    });
}
