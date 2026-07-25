//! Pure extraction contracts for the Orbit graph backend.
//!
//! This crate owns the language-neutral `ExtractedFile` shape and the trait
//! implemented by language-specific tree-sitter extractors. It intentionally
//! avoids storage, async, and filesystem traversal concerns.

use std::path::Path;

/// Raw extraction rows emitted for one source file.
pub mod extracted;
/// Language-specific extractor implementations.
pub mod languages;
/// Stable selector parser shared by graph callers and extractors.
pub mod selector;

pub use extracted::{
    ExtractedFile, RawCommand, RawConfig, RawImport, RawRef, RawRelation, RawString, RawSymbol,
};
pub use selector::{Selector, SelectorParseError};

#[cfg(test)]
mod tests;

/// Extracts graph rows from a single file's bytes.
pub trait Extractor {
    /// Returns the stable language identifier emitted for indexed files.
    fn lang(&self) -> &'static str;

    /// Returns whether this extractor should handle `path`.
    fn supports(&self, path: &Path) -> bool;

    /// Extracts raw graph rows from `bytes` at `path`.
    fn extract(&self, path: &Path, bytes: &[u8]) -> ExtractedFile;
}
