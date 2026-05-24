//! JavaScript tree-sitter extraction.

use std::path::Path;

use tree_sitter::Parser;

use crate::{ExtractedFile, Extractor};

use super::js_ts::{JsTsOptions, extract_file};

/// Extracts JavaScript and JSX source files into raw graph rows.
pub struct JavaScriptExtractor;

impl Extractor for JavaScriptExtractor {
    fn lang(&self) -> &'static str {
        "javascript"
    }

    fn supports(&self, path: &Path) -> bool {
        matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("js" | "jsx")
        )
    }

    fn extract(&self, path: &Path, bytes: &[u8]) -> ExtractedFile {
        let Ok(source) = std::str::from_utf8(bytes) else {
            return ExtractedFile::default();
        };

        let mut parser = Parser::new();
        if parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .is_err()
        {
            return ExtractedFile::default();
        }

        let Some(tree) = parser.parse(source, None) else {
            return ExtractedFile::default();
        };

        extract_file(
            path,
            source,
            tree.root_node(),
            JsTsOptions { type_syntax: false },
        )
    }
}

#[cfg(test)]
#[path = "tests/javascript.rs"]
mod tests;
