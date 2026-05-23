//! File content extractors for the knowledge graph.
//!
//! Extracts structural symbols from source code via tree-sitter (`Language`
//! extractors in `rust.rs`, `python.rs`, …), markdown heading anchors, and
//! file-level source capture for structured config and tabular files.
//!
//! Dispatch key is `FileKind`, which subsumes the prior `Language`-based
//! dispatch under `FileKind::Code(Language)` without changing extractor
//! internals.

mod c;
mod common;
mod config;
mod csharp;
mod go;
mod java;
mod javascript;
mod kotlin;
mod language;
mod markdown;
mod python;
mod ruby;
mod rust;
mod table;
mod typescript;

pub use common::{
    ExtractedExport, ExtractedLeaf, ExtractionResult, compute_source_hash,
    finalize_unique_qualified_names, identity_key, leaf_location, node_id,
};
pub use language::{ConfigFormat, DocFormat, FileKind, Language, TableFormat};

use c::CExtractor;
use config::ConfigExtractor;
use csharp::CSharpExtractor;
use go::GoExtractor;
use java::JavaExtractor;
use javascript::JavaScriptExtractor;
use kotlin::KotlinExtractor;
use markdown::MarkdownExtractor;
use python::PythonExtractor;
use ruby::RubyExtractor;
use rust::RustExtractor;
use table::TableExtractor;
use typescript::TypeScriptExtractor;

/// Trait for file-content extractors.
///
/// Implementors declare the `FileKind` they handle and emit a flat list of
/// `ExtractedLeaf` anchors for that file's content. For code, anchors are
/// symbols; for markdown, anchors are sections. Config and table extractors
/// stay registered to preserve file-level source capture but emit no leaves.
pub trait FileExtractor: Send + Sync {
    fn file_kind(&self) -> FileKind;
    fn extract(&self, source: &str) -> ExtractionResult;
}

/// Registry of available extractors.
pub struct ExtractorRegistry {
    extractors: Vec<Box<dyn FileExtractor>>,
}

impl ExtractorRegistry {
    pub fn new() -> Self {
        Self {
            extractors: vec![
                Box::new(CExtractor),
                Box::new(CSharpExtractor),
                Box::new(RustExtractor),
                Box::new(PythonExtractor),
                Box::new(RubyExtractor),
                Box::new(GoExtractor),
                Box::new(JavaExtractor),
                Box::new(JavaScriptExtractor),
                Box::new(KotlinExtractor),
                Box::new(TypeScriptExtractor::new(Language::TypeScript)),
                Box::new(TypeScriptExtractor::new(Language::Tsx)),
                Box::new(MarkdownExtractor),
                Box::new(ConfigExtractor::new(ConfigFormat::Yaml)),
                Box::new(ConfigExtractor::new(ConfigFormat::Json)),
                Box::new(ConfigExtractor::new(ConfigFormat::Toml)),
                Box::new(TableExtractor::new(TableFormat::Csv)),
                Box::new(TableExtractor::new(TableFormat::Tsv)),
            ],
        }
    }

    pub fn get(&self, kind: FileKind) -> Option<&dyn FileExtractor> {
        self.extractors
            .iter()
            .find(|e| e.file_kind() == kind)
            .map(|e| e.as_ref())
    }
}

impl Default for ExtractorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract leaves from source using the code extractor for `language`.
///
/// Thin compatibility shim for working-graph callers that dispatch on a
/// `Language` directly (not on a `FileKind`). Non-code files do not reach
/// this path — the pipeline layer dispatches by `FileKind`.
pub fn extract_file(source: &str, language: Language) -> ExtractionResult {
    let registry = ExtractorRegistry::new();
    registry
        .get(FileKind::Code(language))
        .map(|e| e.extract(source))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
