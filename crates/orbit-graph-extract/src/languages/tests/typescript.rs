#![allow(missing_docs)]

use std::path::Path;

use crate::Extractor;
use crate::languages::TypeScriptExtractor;

fn extract(source: &str) -> crate::ExtractedFile {
    TypeScriptExtractor.extract(Path::new("src/sample.tsx"), source.as_bytes())
}

#[test]
fn supports_typescript_and_tsx_files() {
    assert!(TypeScriptExtractor.supports(Path::new("src/sample.ts")));
    assert!(TypeScriptExtractor.supports(Path::new("src/sample.tsx")));
    assert!(!TypeScriptExtractor.supports(Path::new("src/sample.js")));
}

#[test]
fn extracts_typescript_symbols_relations_imports_and_refs() {
    let source = r#"
import DefaultWidget, { Service as LocalService, helper } from './service';
import type { Model } from './types';
import './polyfill';
export { helper as exportedHelper } from './service';

interface Renderable {
    render(input: Model): string;
}

type Result<T> = Promise<T>;

class Widget extends DefaultWidget implements Renderable {
    render(input: Model): string {
        this.service.run();
        helper();
        return input.name;
    }
}

function identity<T extends Model>(value: T): Result<T> {
    return Promise.resolve(value);
}
"#;

    let file = extract(source);

    for (kind, name) in [
        ("interface", "Renderable"),
        ("type_alias", "Result"),
        ("class", "Widget"),
        ("method", "render"),
        ("function", "identity"),
    ] {
        assert!(
            file.symbols
                .iter()
                .any(|symbol| symbol.kind == kind && symbol.name == name),
            "missing {kind} {name}"
        );
    }
    assert!(
        file.symbols.iter().all(|symbol| {
            symbol.span_start < symbol.span_end && symbol.span_end <= source.len()
        }),
        "symbols should use byte spans"
    );

    assert!(file.relations.iter().any(|relation| {
        relation.kind == "extends"
            && relation.from_qualified == "Widget"
            && relation.to_qualified == "DefaultWidget"
    }));
    assert!(file.relations.iter().any(|relation| {
        relation.kind == "implements"
            && relation.from_qualified == "Widget"
            && relation.to_qualified == "Renderable"
    }));
    assert!(
        file.relations
            .iter()
            .all(|relation| relation.def_span_end <= source.len()),
        "relations should use byte spans"
    );

    assert!(file.imports.iter().any(|import| {
        import.target_path == "./service"
            && import.target_symbol.as_deref() == Some("DefaultWidget")
    }));
    assert!(file.imports.iter().any(|import| {
        import.target_path == "./service" && import.target_symbol.as_deref() == Some("LocalService")
    }));
    assert!(file.imports.iter().any(|import| {
        import.target_path == "./types" && import.target_symbol.as_deref() == Some("Model")
    }));
    assert!(file.imports.iter().any(|import| {
        import.target_path == "./service"
            && import.target_symbol.as_deref() == Some("exportedHelper")
    }));
    assert!(
        file.imports
            .iter()
            .any(|import| { import.target_path == "./polyfill" && import.target_symbol.is_none() })
    );

    assert!(
        file.refs
            .iter()
            .any(|reference| { reference.kind == "type" && reference.target_name == "Model" })
    );
    assert!(
        file.refs
            .iter()
            .any(|reference| { reference.kind == "type" && reference.target_name == "Promise" })
    );
    assert!(file.refs.iter().any(|reference| {
        reference.kind == "call"
            && reference.target_name == "run"
            && reference.target_qualified.is_none()
            && reference.confidence == "fuzzy_name"
    }));
    assert!(
        file.refs
            .iter()
            .all(|reference| reference.from_span_end <= source.len()),
        "refs should use byte spans"
    );
}
