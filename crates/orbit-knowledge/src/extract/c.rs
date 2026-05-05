use tree_sitter::{Node, Parser};

use super::FileExtractor;
use super::common::{ExtractedLeaf, ExtractionResult, compute_source_hash};
use super::language::{FileKind, Language};

pub struct CExtractor;

impl FileExtractor for CExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Code(Language::C)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .expect("tree-sitter-c");

        let tree = match parser.parse(source, None) {
            Some(t) => t,
            None => return ExtractionResult::default(),
        };

        let mut leaves = Vec::new();
        extract_top_level(tree.root_node(), source, &mut leaves);
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

fn extract_top_level(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }

        match child.kind() {
            "function_definition" => extract_function_definition(child, source, leaves),
            "declaration" => extract_declaration(child, source, leaves),
            "type_definition" => extract_type_definition(child, source, leaves),
            "struct_specifier" | "union_specifier" | "enum_specifier" => {
                extract_tag_specifier(child, source, leaves)
            }
            "preproc_def" | "preproc_function_def" => extract_macro(child, source, leaves),
            "preproc_if" | "preproc_ifdef" | "preproc_elif" | "preproc_elifdef"
            | "preproc_else" => extract_top_level(child, source, leaves),
            _ => {}
        }
    }
}

fn extract_function_definition(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let Some(declarator) = node.child_by_field_name("declarator") else {
        return;
    };
    let Some(name) = declarator_name(declarator, source) else {
        return;
    };

    push_leaf(node, source, leaves, &name, &name, "function");
}

fn extract_declaration(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    if let Some(type_node) = node.child_by_field_name("type") {
        extract_tag_specifier(type_node, source, leaves);
    }

    for declarator in declarators(node) {
        let Some(name) = declarator_name(declarator, source) else {
            continue;
        };

        if is_function_prototype_declarator(declarator, source) {
            // Prototypes and definitions both represent callable C symbols.
            // The kind tag distinguishes header/source declarations from
            // definitions because ExtractedLeaf has no separate signature field.
            push_leaf(node, source, leaves, &name, &name, "function_declaration");
        } else {
            push_leaf(node, source, leaves, &name, &name, "global");
        }
    }
}

fn extract_type_definition(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    if let Some(type_node) = node.child_by_field_name("type") {
        extract_tag_specifier(type_node, source, leaves);
    }

    for declarator in declarators(node) {
        let Some(name) = declarator_name(declarator, source) else {
            continue;
        };
        push_leaf(node, source, leaves, &name, &name, "type_alias");
    }
}

fn extract_tag_specifier(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
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
    push_leaf(node, source, leaves, &name, &name, kind);
}

fn extract_macro(node: Node, source: &str, leaves: &mut Vec<ExtractedLeaf>) {
    let Some(name) = get_name(node, source) else {
        return;
    };
    push_leaf(node, source, leaves, &name, &name, "macro");
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

fn push_leaf(
    node: Node,
    source: &str,
    leaves: &mut Vec<ExtractedLeaf>,
    name: &str,
    qualified_name: &str,
    kind: &str,
) {
    let src = node_source(node, source);
    let start_line = node.start_position().row + 1;
    let line_count = src.lines().count().max(1);
    leaves.push(ExtractedLeaf {
        qualified_name: qualified_name.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
        start_line,
        end_line: start_line + line_count - 1,
        source: src.clone(),
        source_hash: compute_source_hash(&src),
        parent_qualified_name: None,
        children_qualified_names: vec![],
        depth: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_fixture() -> &'static str {
        r#"#define ORBIT_C_LIMIT 16

struct OrbitPacket {
    int id;
    const char *payload;
};

union OrbitValue {
    int as_int;
    float as_float;
};

enum OrbitState {
    ORBIT_IDLE,
    ORBIT_ACTIVE,
};

typedef struct OrbitPacket OrbitPacketAlias;

int orbit_global_counter = 0;

static void orbit_reset(void) {
    orbit_global_counter = 0;
}

int orbit_sum(int left, int right) {
    return left + right;
}"#
    }

    fn header_fixture() -> &'static str {
        r#"#ifndef ORBIT_PACKET_H
#define ORBIT_PACKET_H

#define ORBIT_PACKET_MAGIC 0x51

struct OrbitHeader {
    unsigned version;
};

typedef enum OrbitHeaderKind {
    ORBIT_HEADER_SHORT,
    ORBIT_HEADER_LONG,
} OrbitHeaderKind;

extern int orbit_header_global;

int orbit_parse_header(const char *bytes, unsigned len);
void orbit_emit_header(struct OrbitHeader *header);

#endif"#
    }

    fn leaf<'a>(leaves: &'a [ExtractedLeaf], name: &str, kind: &str) -> &'a ExtractedLeaf {
        leaves
            .iter()
            .find(|leaf| leaf.name == name && leaf.kind == kind)
            .unwrap_or_else(|| panic!("missing {kind} leaf {name}"))
    }

    #[test]
    fn file_kind_is_c() {
        assert_eq!(CExtractor.file_kind(), FileKind::Code(Language::C));
    }

    #[test]
    fn extracts_c_source_symbols() {
        let result = CExtractor.extract(source_fixture());
        let leaves = result.leaves;

        let macro_leaf = leaf(&leaves, "ORBIT_C_LIMIT", "macro");
        assert_eq!(macro_leaf.start_line, 1);
        assert_eq!(macro_leaf.end_line, 1);

        let struct_leaf = leaf(&leaves, "OrbitPacket", "struct");
        assert_eq!(struct_leaf.start_line, 3);
        assert_eq!(struct_leaf.end_line, 6);

        let union_leaf = leaf(&leaves, "OrbitValue", "union");
        assert_eq!(union_leaf.start_line, 8);
        assert_eq!(union_leaf.end_line, 11);

        let enum_leaf = leaf(&leaves, "OrbitState", "enum");
        assert_eq!(enum_leaf.start_line, 13);
        assert_eq!(enum_leaf.end_line, 16);

        let typedef_leaf = leaf(&leaves, "OrbitPacketAlias", "type_alias");
        assert_eq!(typedef_leaf.start_line, 18);
        assert_eq!(typedef_leaf.end_line, 18);

        let global_leaf = leaf(&leaves, "orbit_global_counter", "global");
        assert_eq!(global_leaf.start_line, 20);
        assert_eq!(global_leaf.end_line, 20);

        let reset_leaf = leaf(&leaves, "orbit_reset", "function");
        assert_eq!(reset_leaf.start_line, 22);
        assert_eq!(reset_leaf.end_line, 24);
        assert!(reset_leaf.source.starts_with("static void orbit_reset"));

        let sum_leaf = leaf(&leaves, "orbit_sum", "function");
        assert_eq!(sum_leaf.start_line, 26);
        assert_eq!(sum_leaf.end_line, 28);
    }

    #[test]
    fn extracts_c_header_symbols_and_prototypes() {
        let result = CExtractor.extract(header_fixture());
        let leaves = result.leaves;

        let macro_leaf = leaf(&leaves, "ORBIT_PACKET_MAGIC", "macro");
        assert_eq!(macro_leaf.start_line, 4);
        assert_eq!(macro_leaf.end_line, 4);

        let struct_leaf = leaf(&leaves, "OrbitHeader", "struct");
        assert_eq!(struct_leaf.start_line, 6);
        assert_eq!(struct_leaf.end_line, 8);

        let enum_leaf = leaf(&leaves, "OrbitHeaderKind", "enum");
        assert_eq!(enum_leaf.start_line, 10);
        assert_eq!(enum_leaf.end_line, 13);

        let typedef_leaf = leaf(&leaves, "OrbitHeaderKind", "type_alias");
        assert_eq!(typedef_leaf.start_line, 10);
        assert_eq!(typedef_leaf.end_line, 13);

        let global_leaf = leaf(&leaves, "orbit_header_global", "global");
        assert_eq!(global_leaf.start_line, 15);
        assert_eq!(global_leaf.end_line, 15);

        let parse_leaf = leaf(&leaves, "orbit_parse_header", "function_declaration");
        assert_eq!(parse_leaf.start_line, 17);
        assert_eq!(parse_leaf.end_line, 17);
        assert_eq!(
            parse_leaf.source,
            "int orbit_parse_header(const char *bytes, unsigned len);"
        );

        let emit_leaf = leaf(&leaves, "orbit_emit_header", "function_declaration");
        assert_eq!(emit_leaf.start_line, 18);
        assert_eq!(emit_leaf.end_line, 18);
    }
}
