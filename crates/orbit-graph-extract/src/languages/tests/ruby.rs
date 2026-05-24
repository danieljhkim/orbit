#![allow(missing_docs)]

use std::path::Path;

use crate::Extractor;

use super::RubyExtractor;

fn extract(source: &str) -> crate::ExtractedFile {
    RubyExtractor.extract(Path::new("src/sample.rb"), source.as_bytes())
}

#[test]
fn extracts_symbols_imports_extends_and_fuzzy_calls() {
    let source = r#"
require "json"

module Mix
end

class Widget < Base
  include Mix
  extend Helpers

  def run(worker)
    worker.perform
    helper
  end
end

def helper
end
"#;

    let file = extract(source);

    assert!(RubyExtractor.supports(Path::new("src/sample.rb")));
    assert!(!RubyExtractor.supports(Path::new("src/sample.py")));
    assert!(
        file.symbols.iter().all(|symbol| {
            symbol.span_start < symbol.span_end && symbol.span_end <= source.len()
        })
    );
    assert!(
        file.symbols
            .iter()
            .any(|symbol| { symbol.kind == "module" && symbol.qualified == "Mix" })
    );
    assert!(
        file.symbols
            .iter()
            .any(|symbol| { symbol.kind == "class" && symbol.qualified == "Widget" })
    );
    assert!(file.symbols.iter().any(|symbol| {
        symbol.kind == "method"
            && symbol.qualified == "Widget::run"
            && symbol.parent_symbol.as_deref() == Some("Widget")
    }));
    assert!(
        file.symbols
            .iter()
            .any(|symbol| { symbol.kind == "method" && symbol.qualified == "helper" })
    );

    assert!(
        file.imports
            .iter()
            .any(|import| { import.target_path == "json" && import.target_symbol.is_none() })
    );
    for target in ["Base", "Mix", "Helpers"] {
        assert!(file.relations.iter().any(|relation| {
            relation.from_qualified == "Widget"
                && relation.to_qualified == target
                && relation.kind == "extends"
        }));
    }
    assert!(file.refs.iter().any(|reference| {
        reference.kind == "call"
            && reference.target_name == "perform"
            && reference.target_qualified.is_none()
            && reference.confidence == "fuzzy_name"
    }));
}
