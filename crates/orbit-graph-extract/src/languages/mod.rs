//! Language-specific tree-sitter extractors.

pub mod c;
mod common;
pub mod config;
pub mod csharp;
pub mod java;
pub mod kotlin;
pub mod markdown;
pub mod rust;

pub use c::CExtractor;
pub use config::ConfigExtractor;
pub use csharp::CSharpExtractor;
pub use java::JavaExtractor;
pub use kotlin::KotlinExtractor;
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
        Box::new(JavaExtractor),
        Box::new(KotlinExtractor),
        Box::new(CSharpExtractor),
    ]
}

#[cfg(test)]
mod tests;
