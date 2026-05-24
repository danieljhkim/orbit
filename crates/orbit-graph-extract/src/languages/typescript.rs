//! TypeScript tree-sitter extraction.

use std::path::Path;

use tree_sitter::{Language as TreeSitterLanguage, Parser};

use crate::{ExtractedFile, Extractor};

use super::js_ts::{JsTsOptions, extract_file};

/// Extracts TypeScript and TSX source files into raw graph rows.
pub struct TypeScriptExtractor;

impl Extractor for TypeScriptExtractor {
    fn lang(&self) -> &'static str {
        "typescript"
    }

    fn supports(&self, path: &Path) -> bool {
        matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("ts" | "tsx")
        )
    }

    fn extract(&self, path: &Path, bytes: &[u8]) -> ExtractedFile {
        let Ok(source) = std::str::from_utf8(bytes) else {
            return ExtractedFile::default();
        };

        let mut parser = Parser::new();
        if parser.set_language(&tree_sitter_language(path)).is_err() {
            return ExtractedFile::default();
        }

        let Some(tree) = parser.parse(source, None) else {
            return ExtractedFile::default();
        };

        extract_file(
            path,
            source,
            tree.root_node(),
            JsTsOptions { type_syntax: true },
        )
    }
}

fn tree_sitter_language(path: &Path) -> TreeSitterLanguage {
    if path.extension().and_then(|ext| ext.to_str()) == Some("tsx") {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }
}

#[cfg(test)]
#[path = "tests/typescript.rs"]
mod tests;
