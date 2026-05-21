// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use tree_sitter::{Node, Parser};

use super::FileExtractor;
use super::common::{
    ExtractedExport, ExtractedLeaf, ExtractionResult, compute_source_hash,
    finalize_unique_qualified_names,
};
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
            None => return ExtractionResult::default(),
        };

        let mut leaves = Vec::new();
        let mut exports = Vec::new();
        extract_top_level(tree.root_node(), source, &mut leaves, &mut exports);
        sort_and_dedup_exports(&mut exports);
        finalize_unique_qualified_names(&mut leaves);
        ExtractionResult { leaves, exports }
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

fn extract_top_level(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    exports: &mut Vec<ExtractedExport>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                extract_function(child, source, leaves, None);
                extract_named_public_export(child, source, exports);
            }
            "struct_item" | "enum_item" => {
                extract_struct_like(child, source, leaves);
                extract_named_public_export(child, source, exports);
            }
            "trait_item" => {
                extract_trait(child, source, leaves);
                extract_named_public_export(child, source, exports);
            }
            "impl_item" => extract_impl(child, source, leaves),
            "mod_item" => {
                extract_mod(child, source, leaves);
                extract_named_public_export(child, source, exports);
            }
            "type_item" => {
                extract_type_alias(child, source, leaves);
                extract_named_public_export(child, source, exports);
            }
            "const_item" | "static_item" => {
                extract_const_static(child, source, leaves);
                extract_named_public_export(child, source, exports);
            }
            "use_declaration" => extract_pub_use_exports(child, source, exports),
            _ => {}
        }
    }
}

fn extract_named_public_export(node: Node, source: &str, exports: &mut Vec<ExtractedExport>) {
    if !is_unrestricted_public(node, source) {
        return;
    }
    if let Some(name) = get_name(node, source) {
        exports.push(ExtractedExport {
            name,
            source_path: None,
        });
    }
}

fn extract_pub_use_exports(node: Node, source: &str, exports: &mut Vec<ExtractedExport>) {
    if !is_unrestricted_public(node, source) {
        return;
    }
    let Some(argument) = node.child_by_field_name("argument") else {
        return;
    };
    collect_use_exports(argument, source, &[], exports);
}

fn is_unrestricted_public(node: Node, source: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|child| {
        child.kind() == "visibility_modifier" && node_source(child, source).as_str() == "pub"
    })
}

fn collect_use_exports(
    node: Node,
    source: &str,
    prefix: &[String],
    exports: &mut Vec<ExtractedExport>,
) {
    match node.kind() {
        "scoped_use_list" => {
            let mut next_prefix = prefix.to_vec();
            if let Some(path) = node.child_by_field_name("path") {
                next_prefix.extend(path_segments(path, source));
            }
            if let Some(list) = node.child_by_field_name("list") {
                collect_use_exports(list, source, &next_prefix, exports);
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_use_exports(child, source, prefix, exports);
            }
        }
        "use_as_clause" => {
            let Some(alias) = node.child_by_field_name("alias") else {
                return;
            };
            let Some(path) = node.child_by_field_name("path") else {
                return;
            };
            let mut source_segments = prefix.to_vec();
            source_segments.extend(path_segments(path, source));
            let Some(source_path) = join_segments(&source_segments) else {
                return;
            };
            exports.push(ExtractedExport {
                name: node_source(alias, source),
                source_path: Some(source_path),
            });
        }
        "use_wildcard" => {
            let mut source_segments = prefix.to_vec();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                source_segments.extend(path_segments(child, source));
            }
            let Some(path) = join_segments(&source_segments) else {
                return;
            };
            let source_path = format!("{path}::*");
            exports.push(ExtractedExport {
                name: source_path.clone(),
                source_path: Some(source_path),
            });
        }
        "identifier" | "crate" | "self" | "super" | "scoped_identifier" => {
            let mut source_segments = prefix.to_vec();
            source_segments.extend(path_segments(node, source));
            let Some(source_path) = join_segments(&source_segments) else {
                return;
            };
            let name = export_name(&source_segments);
            exports.push(ExtractedExport {
                name,
                source_path: Some(source_path),
            });
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_use_exports(child, source, prefix, exports);
            }
        }
    }
}

fn path_segments(node: Node, source: &str) -> Vec<String> {
    node_source(node, source)
        .split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn join_segments(segments: &[String]) -> Option<String> {
    if segments.is_empty() {
        None
    } else {
        Some(segments.join("::"))
    }
}

fn export_name(segments: &[String]) -> String {
    match segments.last().map(String::as_str) {
        Some("self") if segments.len() > 1 => segments[segments.len() - 2].clone(),
        Some(name) => name.to_string(),
        None => String::new(),
    }
}

fn sort_and_dedup_exports(exports: &mut Vec<ExtractedExport>) {
    exports.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.source_path.cmp(&right.source_path))
    });
    exports
        .dedup_by(|left, right| left.name == right.name && left.source_path == right.source_path);
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
    let type_name = match node.child_by_field_name("type") {
        Some(n) => n.utf8_text(source.as_bytes()).unwrap_or("").to_string(),
        None => return,
    };

    if type_name.is_empty() {
        return;
    }

    let trait_name = node
        .child_by_field_name("trait")
        .map(|node| node_source(node, source))
        .filter(|name| !name.is_empty());
    let qualified_name = match trait_name {
        Some(trait_name) => format!("<{type_name} as {trait_name}>"),
        None => format!("<{type_name}>"),
    };
    let src = node_source(node, source);
    let mut children = Vec::new();

    // Extract methods from the body (declaration_list)
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "function_item" {
                extract_function(child, source, leaves, Some(&qualified_name));
                if let Some(method_name) = get_name(child, source) {
                    children.push(format!("{qualified_name}::{method_name}"));
                }
            }
        }
    }

    leaves.push(ExtractedLeaf {
        qualified_name: qualified_name.clone(),
        name: type_name,
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

#[cfg(test)]
mod tests;
