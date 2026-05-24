#![allow(missing_docs)]

use std::path::Path;

use crate::Extractor;
use crate::languages::CSharpExtractor;

fn extract(source: &str) -> crate::ExtractedFile {
    CSharpExtractor.extract(Path::new("src/Sample.cs"), source.as_bytes())
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
fn supports_csharp_files() {
    assert_eq!(CSharpExtractor.lang(), "csharp");
    assert!(CSharpExtractor.supports(Path::new("src/Worker.cs")));
    assert!(!CSharpExtractor.supports(Path::new("src/Worker.java")));
}

#[test]
fn extracts_classes_interfaces_imports_relations_generics_and_fuzzy_calls() {
    let source = r#"
using System.Collections.Generic;

namespace Demo;

class Worker<T> : BaseWorker, IWorker, IDisposable
{
    public void Run(Helper helper)
    {
        helper.Execute();
    }
}

interface IWorker
{
    void Run(Helper helper);
}
"#;

    let file = extract(source);

    assert!(file.symbols.iter().any(|symbol| {
        symbol.kind == "class" && symbol.name == "Worker" && symbol.qualified == "Demo::Worker"
    }));
    assert!(
        file.symbols
            .iter()
            .any(|symbol| { symbol.kind == "interface" && symbol.name == "IWorker" })
    );
    assert!(
        file.symbols
            .iter()
            .any(|symbol| symbol.kind == "method" && symbol.name == "Run")
    );
    assert!(
        file.imports
            .iter()
            .any(|import| import.target_path == "System.Collections.Generic")
    );
    assert!(file.relations.iter().any(|relation| {
        relation.kind == "extends"
            && relation.from_qualified == "Demo::Worker"
            && relation.to_qualified == "BaseWorker"
    }));
    assert!(file.relations.iter().any(|relation| {
        relation.kind == "implements"
            && relation.from_qualified == "Demo::Worker"
            && relation.to_qualified == "IWorker"
    }));
    assert!(file.relations.iter().any(|relation| {
        relation.kind == "implements"
            && relation.from_qualified == "Demo::Worker"
            && relation.to_qualified == "IDisposable"
    }));
    assert!(file.refs.iter().any(|reference| {
        reference.kind == "call"
            && reference.target_name == "Execute"
            && reference.target_qualified.is_none()
            && reference.confidence == "fuzzy_name"
    }));
    assert_byte_spans(&file, source);
}
