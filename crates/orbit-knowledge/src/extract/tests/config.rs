#![allow(missing_docs)]

// Tests for extract/config.rs live here as sibling under extract/tests/ per
// docs/design-patterns/test_layout.md. Explicit named imports, no blanket.

use super::super::{ConfigFormat, ExtractionResult, ExtractorRegistry, FileKind};

fn extract(format: ConfigFormat, source: &str) -> ExtractionResult {
    ExtractorRegistry::new()
        .get(FileKind::Config(format))
        .expect("config extractor registered")
        .extract(source)
}

#[test]
fn yaml_files_emit_no_config_key_leaves() {
    let src = "name: orbit\n\
               version: 0.1.0\n\
               nested:\n  child: ignored\n\
               owners:\n  - claude\n  - codex\n";
    let out = extract(ConfigFormat::Yaml, src);
    assert!(out.leaves.is_empty());
}

#[test]
fn json_files_emit_no_config_key_leaves() {
    let src = r#"{"a": 1, "b": {"c": 2}, "d": [1,2]}"#;
    let out = extract(ConfigFormat::Json, src);
    assert!(out.leaves.is_empty());
}

#[test]
fn toml_files_emit_no_config_key_leaves() {
    let src = "name = \"orbit\"\n\
               version = \"0.1.0\"\n\
               [package]\n\
               edition = \"2021\"\n";
    let out = extract(ConfigFormat::Toml, src);
    assert!(out.leaves.is_empty());
}

#[test]
fn malformed_input_still_produces_no_leaves() {
    let out = extract(ConfigFormat::Json, "{not valid");
    assert!(out.leaves.is_empty());
    let out = extract(ConfigFormat::Yaml, "a: b:\n  - [");
    assert!(out.leaves.is_empty());
    let out = extract(ConfigFormat::Toml, "[unclosed");
    assert!(out.leaves.is_empty());
}
