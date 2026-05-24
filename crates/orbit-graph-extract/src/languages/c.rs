//! C tree-sitter extraction (ported/adapted from orbit-knowledge extract/c.rs for ORB-00305).
//!
//! Emits: function / function_declaration / struct / enum / union / type_alias symbols
//! with byte-offset spans. #include populates imports (target_path = header path).
//! No relations (no class hierarchy in C). Macros out of scope per task.

use std::path::Path;

use tree_sitter::{Node, Parser};

use super::common::{dedup_imports, dedup_symbols, normalize_path};
use crate::{ExtractedFile, Extractor, RawImport, RawSymbol};

/// C tree-sitter extractor (functions, structs, includes).
pub struct CExtractor;

impl Extractor for CExtractor {
    fn lang(&self) -> &'static str {
        "c"
    }

    fn supports(&self, path: &Path) -> bool {
        matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("c") | Some("h")
        )
    }

    fn extract(&self, path: &Path, bytes: &[u8]) -> ExtractedFile {
        let Ok(source) = std::str::from_utf8(bytes) else {
            return ExtractedFile::default();
        };

        let mut parser = Parser::new();
        if parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .is_err()
        {
            return ExtractedFile::default();
        }

        let Some(tree) = parser.parse(source, None) else {
            return ExtractedFile::default();
        };

        let mut state = ExtractionState::new(path);
        extract_top_level(tree.root_node(), source, &mut state);
        state.finish()
    }
}

struct ExtractionState {
    file_path: String,
    symbols: Vec<RawSymbol>,
    imports: Vec<RawImport>,
}

impl ExtractionState {
    fn new(path: &Path) -> Self {
        Self {
            file_path: normalize_path(path),
            symbols: Vec::new(),
            imports: Vec::new(),
        }
    }

    fn finish(mut self) -> ExtractedFile {
        dedup_symbols(&mut self.symbols);
        dedup_imports(&mut self.imports);
        ExtractedFile {
            symbols: self.symbols,
            imports: self.imports,
            ..Default::default()
        }
    }

    fn push_symbol(&mut self, node: Node, name: String, kind: &'static str) {
        self.symbols.push(RawSymbol {
            file_path: self.file_path.clone(),
            name: name.clone(),
            qualified: name,
            kind: kind.to_string(),
            span_start: node.start_byte(),
            span_end: node.end_byte(),
            signature: None,
            parent_symbol: None,
        });
    }

    fn push_import(&mut self, target_path: String) {
        self.imports.push(RawImport {
            from_file: self.file_path.clone(),
            target_path,
            target_symbol: None,
        });
    }
}

fn extract_top_level(node: Node, source: &str, state: &mut ExtractionState) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }
        match child.kind() {
            "function_definition" => extract_function_definition(child, source, state),
            "declaration" => extract_declaration(child, source, state),
            "type_definition" => extract_type_definition(child, source, state),
            "struct_specifier" | "union_specifier" | "enum_specifier" => {
                extract_tag_specifier(child, source, state)
            }
            "preproc_include" => extract_include(child, source, state),
            "preproc_if" | "preproc_ifdef" | "preproc_elif" | "preproc_elifdef"
            | "preproc_else" => extract_top_level(child, source, state),
            _ => {}
        }
    }
}

fn extract_include(node: Node, source: &str, state: &mut ExtractionState) {
    // Look for string_literal or system_lib_string child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let text = match child.kind() {
            "string_literal" | "system_lib_string" => {
                let raw = node_text(child, source);
                // strip quotes and <>
                raw.trim_matches(|c| c == '"' || c == '<' || c == '>')
                    .to_string()
            }
            _ => continue,
        };
        if !text.is_empty() {
            state.push_import(text);
        }
    }
}

fn extract_function_definition(node: Node, source: &str, state: &mut ExtractionState) {
    let Some(declarator) = node.child_by_field_name("declarator") else {
        return;
    };
    let Some(name) = declarator_name(declarator, source) else {
        return;
    };
    state.push_symbol(node, name, "function");
}

fn extract_declaration(node: Node, source: &str, state: &mut ExtractionState) {
    if let Some(type_node) = node.child_by_field_name("type") {
        extract_tag_specifier(type_node, source, state);
    }
    for declarator in declarators(node) {
        let Some(name) = declarator_name(declarator, source) else {
            continue;
        };
        if is_function_prototype_declarator(declarator, source) {
            state.push_symbol(node, name, "function_declaration");
        } else {
            state.push_symbol(node, name, "global");
        }
    }
}

fn extract_type_definition(node: Node, source: &str, state: &mut ExtractionState) {
    if let Some(type_node) = node.child_by_field_name("type") {
        extract_tag_specifier(type_node, source, state);
    }
    for declarator in declarators(node) {
        let Some(name) = declarator_name(declarator, source) else {
            continue;
        };
        state.push_symbol(node, name, "type_alias");
    }
}

fn extract_tag_specifier(node: Node, source: &str, state: &mut ExtractionState) {
    let kind = match node.kind() {
        "struct_specifier" => "struct",
        "union_specifier" => "union",
        "enum_specifier" => "enum",
        _ => return,
    };
    if node.child_by_field_name("body").is_none() {
        return;
    }
    let Some(name) = get_name(node, source) else {
        return;
    };
    state.push_symbol(node, name, kind);
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

fn declarators(node: Node) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if is_declarator_kind(child.kind()) {
            nodes.push(child);
        }
    }
    nodes
}

fn declarator_name(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" | "field_identifier" => Some(node_text(node, source)),
        "init_declarator" | "function_declarator" | "array_declarator" | "pointer_declarator" => {
            child_declarator(node).and_then(|child| declarator_name(child, source))
        }
        "attributed_declarator" | "parenthesized_declarator" => {
            first_declarator_child(node).and_then(|child| declarator_name(child, source))
        }
        _ => None,
    }
    .filter(|name| !name.is_empty())
}

fn is_function_prototype_declarator(node: Node, source: &str) -> bool {
    if node.kind() != "function_declarator" {
        return false;
    }
    let Some(inner) = child_declarator(node) else {
        return false;
    };
    declarator_name(inner, source).is_some() && !contains_parenthesized_pointer(inner)
}

fn contains_parenthesized_pointer(node: Node) -> bool {
    if node.kind() == "parenthesized_declarator"
        && first_declarator_child(node)
            .map(|child| child.kind() == "pointer_declarator")
            .unwrap_or(false)
    {
        return true;
    }
    child_declarator(node)
        .map(contains_parenthesized_pointer)
        .unwrap_or(false)
}

fn child_declarator(node: Node) -> Option<Node> {
    node.child_by_field_name("declarator")
        .or_else(|| first_declarator_child(node))
}

fn first_declarator_child(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| is_declarator_kind(child.kind()))
}

fn is_declarator_kind(kind: &str) -> bool {
    matches!(
        kind,
        "array_declarator"
            | "attributed_declarator"
            | "function_declarator"
            | "identifier"
            | "init_declarator"
            | "parenthesized_declarator"
            | "pointer_declarator"
            | "type_identifier"
    )
}
