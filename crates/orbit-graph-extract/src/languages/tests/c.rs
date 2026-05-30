#![allow(missing_docs)]

use std::path::Path;

use crate::Extractor;
use crate::languages::CExtractor;

fn extract(source: &str) -> crate::ExtractedFile {
    extract_at("example.c", source)
}

fn extract_at(path: &str, source: &str) -> crate::ExtractedFile {
    CExtractor.extract(Path::new(path), source.as_bytes())
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
    assert!(
        inc.is_some_and(|import| import.target_symbol.is_none()),
        "missing #include import without target symbol"
    );
}

#[test]
fn c_extractor_emits_fuzzy_call_refs() {
    let source = r#"
#define LOG_EVENT(value) helper(value)

typedef void (*callback_t)(int);
int helper(int value);

void caller(callback_t callback, callback_t indirect) {
    helper(1);
    callback(2);
    LOG_EVENT(3);
    (*indirect)(4);
}
"#;

    let file = extract(source);
    assert!(file.relations.is_empty());
    assert!(file.refs.iter().all(|reference| {
        reference.confidence == "fuzzy_name" && !reference.target_name.is_empty()
    }));
    assert_fuzzy_call_ref(&file, "helper");
    assert_fuzzy_call_ref(&file, "callback");
    assert_fuzzy_call_ref(&file, "LOG_EVENT");
    assert!(
        !file
            .refs
            .iter()
            .any(|reference| reference.target_name == "indirect")
    );

    let header = extract_at(
        "example.h",
        "static inline int header_call(void) { return helper(1); }",
    );
    assert_fuzzy_call_ref(&header, "helper");
}

fn assert_fuzzy_call_ref(file: &crate::ExtractedFile, target_name: &str) {
    assert!(file.refs.iter().any(|reference| {
        reference.kind == "call"
            && reference.target_name == target_name
            && reference.target_qualified.is_none()
            && reference.confidence == "fuzzy_name"
            && reference.from_span_start < reference.from_span_end
    }));
}
