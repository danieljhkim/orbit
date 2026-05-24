#![allow(missing_docs)]

use std::path::Path;

use crate::Extractor;
use crate::languages::JavaScriptExtractor;

fn extract(source: &str) -> crate::ExtractedFile {
    JavaScriptExtractor.extract(Path::new("src/sample.jsx"), source.as_bytes())
}

#[test]
fn supports_javascript_and_jsx_files() {
    assert!(JavaScriptExtractor.supports(Path::new("src/sample.js")));
    assert!(JavaScriptExtractor.supports(Path::new("src/sample.jsx")));
    assert!(!JavaScriptExtractor.supports(Path::new("src/sample.ts")));
}

#[test]
fn extracts_classes_methods_imports_reexports_and_calls() {
    let source = r#"
import DefaultThing, { helper as runHelper, value } from './helpers';
import './setup';
export { runHelper as exportedRun } from './helpers';
export * from './more';

class Widget extends BaseWidget {
    render() {
        this.service.run();
        runHelper();
    }
}

const makeWidget = () => new Widget();
"#;

    let file = extract(source);

    assert!(file.symbols.iter().any(|symbol| {
        symbol.kind == "class" && symbol.name == "Widget" && symbol.qualified == "Widget"
    }));
    assert!(file.symbols.iter().any(|symbol| {
        symbol.kind == "method"
            && symbol.name == "render"
            && symbol.qualified == "Widget::render"
            && symbol.parent_symbol.as_deref() == Some("Widget")
    }));
    assert!(
        file.symbols
            .iter()
            .any(|symbol| { symbol.kind == "function" && symbol.name == "makeWidget" })
    );
    assert!(
        file.symbols.iter().all(|symbol| {
            symbol.span_start < symbol.span_end && symbol.span_end <= source.len()
        }),
        "symbols should use byte spans"
    );

    assert!(file.relations.iter().any(|relation| {
        relation.kind == "extends"
            && relation.from_qualified == "Widget"
            && relation.to_qualified == "BaseWidget"
            && relation.def_span_end <= source.len()
    }));

    assert!(file.imports.iter().any(|import| {
        import.target_path == "./helpers" && import.target_symbol.as_deref() == Some("DefaultThing")
    }));
    assert!(file.imports.iter().any(|import| {
        import.target_path == "./helpers" && import.target_symbol.as_deref() == Some("runHelper")
    }));
    assert!(file.imports.iter().any(|import| {
        import.target_path == "./helpers" && import.target_symbol.as_deref() == Some("exportedRun")
    }));
    assert!(
        file.imports
            .iter()
            .any(|import| { import.target_path == "./setup" && import.target_symbol.is_none() })
    );
    assert!(
        file.imports
            .iter()
            .any(|import| { import.target_path == "./more" && import.target_symbol.is_none() })
    );

    assert!(file.refs.iter().any(|reference| {
        reference.kind == "call"
            && reference.target_name == "run"
            && reference.target_qualified.is_none()
            && reference.confidence == "fuzzy_name"
    }));
    assert!(file.refs.iter().any(|reference| {
        reference.kind == "use"
            && reference.target_name == "runHelper"
            && reference.confidence == "import_resolved"
    }));
    assert!(
        file.refs
            .iter()
            .all(|reference| reference.from_span_end <= source.len()),
        "refs should use byte spans"
    );
}
