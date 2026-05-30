#![allow(missing_docs)]

use std::path::Path;

use crate::Extractor;
use crate::languages::JavaExtractor;

fn extract(source: &str) -> crate::ExtractedFile {
    JavaExtractor.extract(Path::new("src/sample.java"), source.as_bytes())
}

fn assert_byte_spans(file: &crate::ExtractedFile, source: &str) {
    assert!(
        file.symbols
            .iter()
            .all(|symbol| symbol.span_start < symbol.span_end && symbol.span_end <= source.len())
    );
    assert!(file.relations.iter().all(|relation| {
        relation.def_span_start < relation.def_span_end && relation.def_span_end <= source.len()
    }));
    assert!(file.refs.iter().all(|reference| {
        reference.from_span_start < reference.from_span_end
            && reference.from_span_end <= source.len()
    }));
}

#[test]
fn supports_java_files() {
    assert_eq!(JavaExtractor.lang(), "java");
    assert!(JavaExtractor.supports(Path::new("src/Worker.java")));
    assert!(!JavaExtractor.supports(Path::new("src/Worker.kt")));
}

#[test]
fn extracts_classes_interfaces_imports_relations_generics_and_fuzzy_calls() {
    let source = r#"
package demo;

import java.util.List;

class Worker<T> extends BaseWorker implements Runnable, Closeable {
    void run(Helper helper) {
        helper.execute();
    }
}

interface Closeable {
    void close();
}
"#;

    let file = extract(source);

    assert!(file.symbols.iter().any(|symbol| {
        symbol.kind == "class" && symbol.name == "Worker" && symbol.qualified == "Worker"
    }));
    assert!(
        file.symbols
            .iter()
            .any(|symbol| { symbol.kind == "interface" && symbol.name == "Closeable" })
    );
    assert!(
        file.symbols
            .iter()
            .any(|symbol| symbol.kind == "method" && symbol.name == "run")
    );
    assert!(
        file.imports
            .iter()
            .any(|import| import.target_path == "java.util.List")
    );
    assert!(file.relations.iter().any(|relation| {
        relation.kind == "extends"
            && relation.from_qualified == "Worker"
            && relation.to_qualified == "BaseWorker"
    }));
    assert!(file.relations.iter().any(|relation| {
        relation.kind == "implements"
            && relation.from_qualified == "Worker"
            && relation.to_qualified == "Runnable"
    }));
    assert!(file.relations.iter().any(|relation| {
        relation.kind == "implements"
            && relation.from_qualified == "Worker"
            && relation.to_qualified == "Closeable"
    }));
    assert!(file.refs.iter().any(|reference| {
        reference.kind == "call"
            && reference.target_name == "execute"
            && reference.target_qualified.is_none()
            && reference.confidence == "fuzzy_name"
    }));
    assert_byte_spans(&file, source);
}
