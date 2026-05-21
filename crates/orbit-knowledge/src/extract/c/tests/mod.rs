#![allow(missing_docs)]

use super::super::*;

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
