use std::collections::HashMap;

use tree_sitter::{Node, Parser};

use super::FileExtractor;
use super::common::{ExtractedLeaf, ExtractionResult, compute_source_hash};
use super::language::{FileKind, Language};

pub struct GoExtractor;

impl FileExtractor for GoExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Code(Language::Go)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .expect("tree-sitter-go");

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
    let mut type_indices: HashMap<String, usize> = HashMap::new();
    let mut pending_children: HashMap<String, Vec<String>> = HashMap::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => extract_function(child, source, leaves),
            "method_declaration" => {
                if let Some((receiver, qualified_name)) = extract_method(child, source, leaves) {
                    if let Some(index) = type_indices.get(&receiver).copied() {
                        leaves[index].children_qualified_names.push(qualified_name);
                    } else {
                        pending_children
                            .entry(receiver)
                            .or_default()
                            .push(qualified_name);
                    }
                }
            }
            "type_declaration" => extract_type_declaration(
                child,
                source,
                leaves,
                &mut type_indices,
                &mut pending_children,
            ),
            "const_declaration" => extract_binding_declaration(child, source, leaves, "const_spec"),
            "var_declaration" => extract_binding_declaration(child, source, leaves, "var_spec"),
            _ => {}
        }
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
        depth: None,
    });
}

fn extract_method(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
) -> Option<(String, String)> {
    let receiver = receiver_type_name(node, source)?;
    let name = get_name(node, source)?;
    let qualified_name = format!("{receiver}::{name}");
    let src = node_source(node, source);

    leaves.push(ExtractedLeaf {
        qualified_name: qualified_name.clone(),
        name,
        kind: "method".to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: Some(receiver.clone()),
        children_qualified_names: vec![],
        depth: None,
    });

    Some((receiver, qualified_name))
}

fn extract_type_declaration(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    type_indices: &mut HashMap<String, usize>,
    pending_children: &mut HashMap<String, Vec<String>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "type_spec" {
            continue;
        }

        let name = match get_name(child, source) {
            Some(name) => name,
            None => continue,
        };

        let mut children = pending_children.remove(&name).unwrap_or_default();
        let kind = match child.child_by_field_name("type") {
            Some(type_node) if type_node.kind() == "interface_type" => {
                children.extend(extract_interface_methods(type_node, &name, source, leaves));
                "interface"
            }
            _ => "struct",
        };

        let src = node_source(child, source);
        leaves.push(ExtractedLeaf {
            qualified_name: name.clone(),
            name: name.clone(),
            kind: kind.to_string(),
            start_line: child.start_position().row + 1,
            end_line: child.end_position().row + 1,
            source: src.clone(),
            source_hash: compute_source_hash(&src),
            parent_qualified_name: None,
            children_qualified_names: children,
            depth: None,
        });

        type_indices.insert(name, leaves.len() - 1);
    }
}

fn extract_interface_methods(
    interface_type: Node,
    parent: &str,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
) -> Vec<String> {
    let mut children = Vec::new();
    let mut cursor = interface_type.walk();

    for child in interface_type.children(&mut cursor) {
        if child.kind() != "method_elem" {
            continue;
        }

        let name = match get_name(child, source) {
            Some(name) => name,
            None => continue,
        };

        let qualified_name = format!("{parent}::{name}");
        let src = node_source(child, source);
        children.push(qualified_name.clone());

        leaves.push(ExtractedLeaf {
            qualified_name,
            name,
            kind: "method".to_string(),
            start_line: child.start_position().row + 1,
            end_line: child.end_position().row + 1,
            source: src.clone(),
            source_hash: compute_source_hash(&src),
            parent_qualified_name: Some(parent.to_string()),
            children_qualified_names: vec![],
            depth: None,
        });
    }

    children
}

fn extract_binding_declaration(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    spec_kind: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != spec_kind {
            continue;
        }

        let name = match get_name(child, source) {
            Some(name) => name,
            None => continue,
        };

        let src = node_source(child, source);
        leaves.push(ExtractedLeaf {
            qualified_name: name.clone(),
            name,
            kind: "field".to_string(),
            start_line: child.start_position().row + 1,
            end_line: child.end_position().row + 1,
            source: src.clone(),
            source_hash: compute_source_hash(&src),
            parent_qualified_name: None,
            children_qualified_names: vec![],
            depth: None,
        });
    }
}

fn receiver_type_name(node: Node, source: &str) -> Option<String> {
    let receiver = node.child_by_field_name("receiver")?;
    let mut cursor = receiver.walk();

    let declaration = receiver
        .children(&mut cursor)
        .find(|child| child.kind() == "parameter_declaration")?;
    let receiver_type = declaration.child_by_field_name("type")?;
    let raw = receiver_type
        .utf8_text(source.as_bytes())
        .unwrap_or("")
        .trim();

    Some(raw.trim_start_matches('*').trim().to_string())
}
