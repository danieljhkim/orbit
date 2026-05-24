#![allow(missing_docs)]

use std::path::Path;

use crate::Extractor;
use crate::languages::RustExtractor;

fn extract(source: &str) -> crate::ExtractedFile {
    RustExtractor.extract(Path::new("src/sample.rs"), source.as_bytes())
}

fn symbol_kinds(file: &crate::ExtractedFile) -> Vec<&str> {
    file.symbols
        .iter()
        .map(|symbol| symbol.kind.as_str())
        .collect()
}

#[test]
fn extracts_required_symbol_kinds_with_byte_spans() {
    let source = r#"
const LIMIT: usize = 3;
type Name = String;
struct Widget;
enum Mode { Fast }
trait Render {
    fn render(&self) -> String {
        String::new()
    }
}
impl Render for Widget {
    fn render(&self) -> String {
        helper()
    }
}
fn helper() -> String {
    String::new()
}
#[test]
fn helper_test() {}
mod nested {
    pub fn child() {}
}
"#;
    let file = extract(source);
    let kinds = symbol_kinds(&file);

    for kind in [
        "const",
        "type_alias",
        "struct",
        "enum",
        "trait",
        "impl",
        "method",
        "function",
        "test",
        "module",
    ] {
        assert!(kinds.contains(&kind), "missing symbol kind {kind}");
    }
    assert!(
        file.symbols.iter().all(|symbol| {
            symbol.span_start < symbol.span_end && symbol.span_end <= source.len()
        })
    );
}

#[test]
fn records_trait_impl_as_relation() {
    let file = extract(
        r#"
trait Render {}
struct Widget;
impl Render for Widget {}
"#,
    );

    let relation = file
        .relations
        .iter()
        .find(|relation| relation.kind == "impl")
        .expect("impl relation");
    assert_eq!(relation.from_qualified, "Widget");
    assert_eq!(relation.to_qualified, "Render");
    assert_eq!(relation.confidence, "exact");
}

#[test]
fn records_trait_bounds_and_type_uses_as_refs() {
    let file = extract(
        r#"
use std::fmt::Display;

struct Boxed<T: Display> {
    value: Vec<T>,
}

fn render<T>(value: T) -> String
where
    T: Display,
{
    String::new()
}
"#,
    );

    assert!(file.refs.iter().any(|reference| {
        reference.kind == "trait_bound" && reference.target_name == "Display"
    }));
    assert!(
        file.refs
            .iter()
            .any(|reference| reference.kind == "type" && reference.target_name == "Vec")
    );
    assert!(
        !file.relations.iter().any(|relation| {
            relation.to_qualified.ends_with("Display") && relation.kind == "impl"
        })
    );
}

#[test]
fn records_use_imports_and_refs() {
    let file = extract(
        r#"
use std::fmt::Display;
use crate::task::{Task, TaskId as Id};
"#,
    );

    assert!(file.imports.iter().any(|import| {
        import.target_path == "std::fmt" && import.target_symbol.as_deref() == Some("Display")
    }));
    assert!(file.imports.iter().any(|import| {
        import.target_path == "crate::task" && import.target_symbol.as_deref() == Some("Id")
    }));
    assert!(file.refs.iter().any(|reference| {
        reference.kind == "use"
            && reference.target_name == "Display"
            && reference.target_qualified.as_deref() == Some("std::fmt::Display")
    }));
}

#[test]
fn qualifies_nested_module_symbols() {
    let file = extract(
        r#"
mod outer {
    mod inner {
        pub struct Thing;
        pub fn build() -> Thing {
            Thing
        }
    }
}
"#,
    );

    assert!(
        file.symbols
            .iter()
            .any(|symbol| { symbol.kind == "module" && symbol.qualified == "outer::inner" })
    );
    assert!(
        file.symbols
            .iter()
            .any(|symbol| { symbol.kind == "struct" && symbol.qualified == "outer::inner::Thing" })
    );
    assert!(
        file.symbols.iter().any(|symbol| {
            symbol.kind == "function" && symbol.qualified == "outer::inner::build"
        })
    );
}

#[test]
fn ambiguous_method_call_lowers_to_fuzzy_name() {
    let file = extract(
        r#"
fn drive(runner: Runner) {
    runner.run();
}
"#,
    );

    assert!(file.refs.iter().any(|reference| {
        reference.kind == "call"
            && reference.target_name == "run"
            && reference.target_qualified.is_none()
            && reference.confidence == "fuzzy_name"
    }));
}
