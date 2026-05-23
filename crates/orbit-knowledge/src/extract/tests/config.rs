#![allow(missing_docs)]

// Tests for extract/config.rs live here as sibling under extract/tests/ per
// docs/design-patterns/test_layout.md. Explicit named imports, no blanket.

use super::super::FileExtractor;
use super::super::config::ConfigExtractor;
use super::super::language::ConfigFormat;

#[test]
fn yaml_files_emit_no_config_key_leaves() {
    let src = "name: orbit\n\
               version: 0.1.0\n\
               nested:\n  child: ignored\n\
               owners:\n  - claude\n  - codex\n";
    let out = ConfigExtractor::new(ConfigFormat::Yaml).extract(src);
    assert!(out.leaves.is_empty());
}

#[test]
fn json_files_emit_no_config_key_leaves() {
    let src = r#"{"a": 1, "b": {"c": 2}, "d": [1,2]}"#;
    let out = ConfigExtractor::new(ConfigFormat::Json).extract(src);
    assert!(out.leaves.is_empty());
}

#[test]
fn toml_files_emit_no_config_key_leaves() {
    let src = "name = \"orbit\"\n\
               version = \"0.1.0\"\n\
               [package]\n\
               edition = \"2021\"\n";
    let out = ConfigExtractor::new(ConfigFormat::Toml).extract(src);
    assert!(out.leaves.is_empty());
}

#[test]
fn malformed_input_still_produces_no_leaves() {
    let out = ConfigExtractor::new(ConfigFormat::Json).extract("{not valid");
    assert!(out.leaves.is_empty());
    let out = ConfigExtractor::new(ConfigFormat::Yaml).extract("a: b:\n  - [");
    assert!(out.leaves.is_empty());
    let out = ConfigExtractor::new(ConfigFormat::Toml).extract("[unclosed");
    assert!(out.leaves.is_empty());
}
