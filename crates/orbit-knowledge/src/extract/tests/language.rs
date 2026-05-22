#![allow(missing_docs)]

// Tests for extract/language.rs live here as sibling under extract/tests/ per
// docs/design-patterns/test_layout.md. Explicit named imports, no blanket.

use super::super::{ConfigFormat, DocFormat, FileKind, Language, TableFormat};

#[test]
fn from_extension_classifies_code() {
    assert_eq!(FileKind::from_extension("c"), FileKind::Code(Language::C));
    assert_eq!(FileKind::from_extension("h"), FileKind::Code(Language::C));
    assert_eq!(
        FileKind::from_extension("cs"),
        FileKind::Code(Language::CSharp)
    );
    assert_eq!(
        FileKind::from_extension("rs"),
        FileKind::Code(Language::Rust)
    );
    assert_eq!(
        FileKind::from_extension("py"),
        FileKind::Code(Language::Python)
    );
    assert_eq!(FileKind::from_extension("go"), FileKind::Code(Language::Go));
    assert_eq!(
        FileKind::from_extension("java"),
        FileKind::Code(Language::Java)
    );
    assert_eq!(
        FileKind::from_extension("js"),
        FileKind::Code(Language::JavaScript)
    );
    assert_eq!(
        FileKind::from_extension("jsx"),
        FileKind::Code(Language::JavaScript)
    );
    assert_eq!(
        FileKind::from_extension("mjs"),
        FileKind::Code(Language::JavaScript)
    );
    assert_eq!(
        FileKind::from_extension("cjs"),
        FileKind::Code(Language::JavaScript)
    );
    assert_eq!(
        FileKind::from_extension("kt"),
        FileKind::Code(Language::Kotlin)
    );
    assert_eq!(
        FileKind::from_extension("kts"),
        FileKind::Code(Language::Kotlin)
    );
    assert_eq!(
        FileKind::from_extension("ts"),
        FileKind::Code(Language::TypeScript)
    );
    assert_eq!(
        FileKind::from_extension("mts"),
        FileKind::Code(Language::TypeScript)
    );
    assert_eq!(
        FileKind::from_extension("cts"),
        FileKind::Code(Language::TypeScript)
    );
    assert_eq!(
        FileKind::from_extension("tsx"),
        FileKind::Code(Language::Tsx)
    );
    assert_eq!(
        FileKind::from_extension("rb"),
        FileKind::Code(Language::Ruby)
    );
    assert_eq!(
        FileKind::from_extension("rake"),
        FileKind::Code(Language::Ruby)
    );
    assert_eq!(
        FileKind::from_extension("gemspec"),
        FileKind::Code(Language::Ruby)
    );
    assert_eq!(FileKind::from_extension("ts").as_str(), "typescript");
    assert_eq!(FileKind::from_extension("tsx").as_str(), "tsx");
    assert_eq!(FileKind::from_extension("kt").as_str(), "kotlin");
    assert_eq!(FileKind::from_extension("h").as_str(), "c");
    assert_eq!(FileKind::from_extension("cs").as_str(), "csharp");
    assert_eq!(FileKind::from_extension("rb").as_str(), "ruby");
}

#[test]
fn from_extension_classifies_docs() {
    assert_eq!(
        FileKind::from_extension("md"),
        FileKind::Doc(DocFormat::Markdown)
    );
}

#[test]
fn from_extension_classifies_config() {
    assert_eq!(
        FileKind::from_extension("yaml"),
        FileKind::Config(ConfigFormat::Yaml)
    );
    assert_eq!(
        FileKind::from_extension("yml"),
        FileKind::Config(ConfigFormat::Yaml)
    );
    assert_eq!(
        FileKind::from_extension("json"),
        FileKind::Config(ConfigFormat::Json)
    );
    assert_eq!(
        FileKind::from_extension("toml"),
        FileKind::Config(ConfigFormat::Toml)
    );
}

#[test]
fn from_extension_classifies_tables() {
    assert_eq!(
        FileKind::from_extension("csv"),
        FileKind::Table(TableFormat::Csv)
    );
    assert_eq!(
        FileKind::from_extension("tsv"),
        FileKind::Table(TableFormat::Tsv)
    );
}

#[test]
fn from_extension_returns_unknown_for_unrecognized() {
    assert_eq!(FileKind::from_extension("csx"), FileKind::Unknown);
    assert_eq!(FileKind::from_extension("cshtml"), FileKind::Unknown);
    assert_eq!(FileKind::from_extension("xyz"), FileKind::Unknown);
    assert_eq!(FileKind::from_extension(""), FileKind::Unknown);
}

#[test]
fn is_extractable_gates_unknown() {
    assert!(FileKind::from_extension("md").is_extractable());
    assert!(!FileKind::from_extension("xyz").is_extractable());
}
