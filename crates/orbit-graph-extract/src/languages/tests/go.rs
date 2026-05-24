#![allow(missing_docs)]

use std::path::Path;

use crate::Extractor;

use super::GoExtractor;

fn extract(source: &str) -> crate::ExtractedFile {
    GoExtractor.extract(Path::new("src/sample.go"), source.as_bytes())
}

#[test]
fn extracts_symbols_imports_refs_without_implements_relations() {
    let source = r#"
package sample

import (
    "fmt"
    log "github.com/acme/log"
)

type Widget struct {
    Name string
}

type Runner interface {
    Run() error
}

func (w *Widget) Run() error {
    fmt.Println(w.Name)
    return nil
}

func Build(r Runner) {
    r.Run()
    log.Info("built")
}
"#;

    let file = extract(source);

    assert!(GoExtractor.supports(Path::new("src/sample.go")));
    assert!(!GoExtractor.supports(Path::new("src/sample.rb")));
    assert!(
        file.symbols.iter().all(|symbol| {
            symbol.span_start < symbol.span_end && symbol.span_end <= source.len()
        })
    );
    assert!(
        file.symbols
            .iter()
            .any(|symbol| { symbol.kind == "struct" && symbol.qualified == "Widget" })
    );
    assert!(
        file.symbols
            .iter()
            .any(|symbol| { symbol.kind == "interface" && symbol.qualified == "Runner" })
    );
    assert!(file.symbols.iter().any(|symbol| {
        symbol.kind == "method"
            && symbol.qualified == "Widget::Run"
            && symbol.parent_symbol.as_deref() == Some("Widget")
    }));
    assert!(
        file.symbols
            .iter()
            .any(|symbol| { symbol.kind == "function" && symbol.qualified == "Build" })
    );

    assert!(
        file.imports
            .iter()
            .any(|import| { import.target_path == "fmt" && import.target_symbol.is_none() })
    );
    assert!(file.imports.iter().any(|import| {
        import.target_path == "github.com/acme/log"
            && import.target_symbol.as_deref() == Some("log")
    }));
    assert!(file.relations.is_empty());
    assert!(file.refs.iter().any(|reference| {
        reference.kind == "call"
            && reference.target_name == "Run"
            && reference.target_qualified.is_none()
            && reference.confidence == "fuzzy_name"
    }));
    assert!(file.refs.iter().any(|reference| {
        reference.kind == "call"
            && reference.target_name == "Info"
            && reference.target_qualified.as_deref() == Some("github.com/acme/log.Info")
            && reference.confidence == "import_resolved"
    }));
}
