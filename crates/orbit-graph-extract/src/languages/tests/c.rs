#![allow(missing_docs)]

use std::path::Path;

use crate::Extractor;
use crate::languages::CExtractor;

fn extract(source: &str) -> crate::ExtractedFile {
    CExtractor.extract(Path::new("example.c"), source.as_bytes())
}

#[test]
fn extracts_function_declaration_vs_definition_and_struct_typedef() {
    let source = r#"#include "foo.h"

struct Widget {
    int x;
};

typedef struct Gadget {
    char *name;
} Gadget;

int add(int a, int b) { return a + b; }

void proto(int x);
"#;
    let file = extract(source);
    let kinds: Vec<&str> = file.symbols.iter().map(|s| s.kind.as_str()).collect();
    assert!(kinds.contains(&"struct"), "missing struct");
    assert!(kinds.contains(&"type_alias"), "missing typedef/type_alias");
    assert!(kinds.contains(&"function"), "missing function def");
    assert!(
        kinds.contains(&"function_declaration"),
        "missing prototype decl"
    );

    // byte spans
    assert!(
        file.symbols
            .iter()
            .all(|s| s.span_start < s.span_end && s.span_end <= source.len())
    );

    // imports
    let inc = file
        .imports
        .iter()
        .find(|i| i.target_path.contains("foo.h"));
    assert!(inc.is_some(), "missing #include import");
    assert!(inc.unwrap().target_symbol.is_none());
}

#[test]
fn c_extractor_emits_no_relations_or_refs() {
    let file = extract("struct S { int f; }; int f(void) {}");
    assert!(file.relations.is_empty());
    assert!(file.refs.is_empty());
}
