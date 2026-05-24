#![allow(missing_docs)]

use std::path::Path;

use crate::Extractor;

use super::PythonExtractor;

fn extract(source: &str) -> crate::ExtractedFile {
    PythonExtractor.extract(Path::new("src/sample.py"), source.as_bytes())
}

#[test]
fn extracts_symbols_imports_extends_and_fuzzy_calls() {
    let source = r#"
import os.path
from pkg.service import Service as Svc

class Base:
    pass

class Widget(Base, mixins.Renderable):
    def run(self, worker: Svc) -> str:
        worker.perform()
        helper()

def helper():
    return "ok"
"#;

    let file = extract(source);

    assert!(PythonExtractor.supports(Path::new("src/sample.py")));
    assert!(!PythonExtractor.supports(Path::new("src/sample.go")));
    assert!(
        file.symbols.iter().all(|symbol| {
            symbol.span_start < symbol.span_end && symbol.span_end <= source.len()
        })
    );
    assert!(
        file.symbols
            .iter()
            .any(|symbol| { symbol.kind == "class" && symbol.qualified == "Widget" })
    );
    assert!(file.symbols.iter().any(|symbol| {
        symbol.kind == "method"
            && symbol.qualified == "Widget.run"
            && symbol.parent_symbol.as_deref() == Some("Widget")
    }));
    assert!(
        file.symbols
            .iter()
            .any(|symbol| { symbol.kind == "function" && symbol.qualified == "helper" })
    );

    assert!(
        file.imports
            .iter()
            .any(|import| { import.target_path == "os.path" && import.target_symbol.is_none() })
    );
    assert!(file.imports.iter().any(|import| {
        import.target_path == "pkg.service" && import.target_symbol.as_deref() == Some("Svc")
    }));
    assert!(file.relations.iter().any(|relation| {
        relation.from_qualified == "Widget"
            && relation.to_qualified == "Base"
            && relation.kind == "extends"
    }));
    assert!(file.refs.iter().any(|reference| {
        reference.kind == "call"
            && reference.target_name == "perform"
            && reference.target_qualified.is_none()
            && reference.confidence == "fuzzy_name"
    }));
}
