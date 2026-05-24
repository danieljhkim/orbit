//! Language-specific tree-sitter extractors.

mod common;
pub mod c;
pub mod config;
pub mod markdown;
pub mod rust;

pub use c::CExtractor;
pub use config::ConfigExtractor;
pub use markdown::MarkdownExtractor;
pub use rust::RustExtractor;

use crate::Extractor;

/// Returns the language extractors registered in this crate.
pub fn extractors() -> Vec<Box<dyn Extractor>> {
    vec![
        Box::new(RustExtractor),
        Box::new(CExtractor),
        Box::new(MarkdownExtractor),
        Box::new(ConfigExtractor),
    ]
}

#[cfg(test)]
mod tests;
