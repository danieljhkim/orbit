//! Tree-sitter based source code extractors.
//!
//! Extracts structural information (functions, classes, structs, traits, etc.)
//! from source files using tree-sitter grammars.

mod common;
mod go;
mod java;
mod javascript;
mod language;
mod python;
mod rust;

pub use common::{
    ExtractedLeaf, ExtractionResult, compute_source_hash, identity_key, leaf_location, node_id,
};
pub use language::Language;

use go::GoExtractor;
use java::JavaExtractor;
use javascript::JavaScriptExtractor;
use python::PythonExtractor;
use rust::RustExtractor;

/// Trait for language-specific source code extractors.
pub trait LanguageExtractor: Send + Sync {
    fn language(&self) -> &str;
    fn extract(&self, source: &str) -> ExtractionResult;
}

/// Registry of available extractors.
pub struct ExtractorRegistry {
    extractors: Vec<Box<dyn LanguageExtractor>>,
}

impl ExtractorRegistry {
    pub fn new() -> Self {
        Self {
            extractors: vec![
                Box::new(RustExtractor),
                Box::new(PythonExtractor),
                Box::new(GoExtractor),
                Box::new(JavaExtractor),
                Box::new(JavaScriptExtractor),
            ],
        }
    }

    pub fn get(&self, language: Language) -> Option<&dyn LanguageExtractor> {
        let lang_str = language.as_str();
        self.extractors
            .iter()
            .find(|e| e.language() == lang_str)
            .map(|e| e.as_ref())
    }
}

impl Default for ExtractorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract all leaves from source code for the given language.
pub fn extract_file(source: &str, language: Language) -> ExtractionResult {
    let registry = ExtractorRegistry::new();
    registry
        .get(language)
        .map(|e| e.extract(source))
        .unwrap_or(ExtractionResult { leaves: vec![] })
}
