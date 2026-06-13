use std::collections::HashSet;

use tree_sitter::{Node, Parser};

use crate::error::KnowledgeError;

pub(crate) fn rust_callee_names(source: &str) -> Result<HashSet<String>, KnowledgeError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| KnowledgeError::invalid_data(format!("tree-sitter init: {e}")))?;

    let Some(tree) = parser.parse(source, None) else {
        return Ok(HashSet::new());
    };
    let mut callees = HashSet::new();
    collect_callees(tree.root_node(), source, &mut callees);
    Ok(callees)
}

fn collect_callees(node: Node<'_>, source: &str, out: &mut HashSet<String>) {
    match node.kind() {
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function")
                && let Some(name) = trailing_ident(func, source)
            {
                out.insert(name);
            }
        }
        "method_call_expression" => {
            if let Some(method) = node.child_by_field_name("method")
                && let Some(name) = ident_text(method, source)
            {
                out.insert(name);
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_callees(child, source, out);
    }
}

fn trailing_ident(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" => ident_text(node, source),
        "scoped_identifier" => node
            .child_by_field_name("name")
            .and_then(|n| ident_text(n, source)),
        "field_expression" => node
            .child_by_field_name("field")
            .and_then(|n| ident_text(n, source)),
        _ => {
            let mut last: Option<String> = None;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(ident) = trailing_ident(child, source) {
                    last = Some(ident);
                }
            }
            last
        }
    }
}

fn ident_text(node: Node<'_>, source: &str) -> Option<String> {
    let text = source.get(node.start_byte()..node.end_byte())?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}
