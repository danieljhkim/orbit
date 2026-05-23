//! Query-side retrieval primitives for the workspace-local semantic index.

pub mod bm25;
pub mod cosine;
pub mod fuse;
pub mod rollup;

pub use bm25::{Bm25Hit, bm25_top_k, snippet_for_hit};
pub use cosine::{CosineHit, cosine_top_k};
pub use fuse::{FusedCandidate, reciprocal_rank_fusion};
pub use rollup::rollup_to_tasks;

#[cfg(test)]
mod tests;
