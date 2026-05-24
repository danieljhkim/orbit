//! Language-specific tree-sitter extractors.

pub mod rust;

pub use rust::RustExtractor;

use crate::Extractor;

/// Returns the language extractors registered in this crate.
pub fn extractors() -> Vec<Box<dyn Extractor>> {
    vec![Box::new(RustExtractor)]
}

#[cfg(test)]
mod tests;
