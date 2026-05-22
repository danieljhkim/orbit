#![deny(clippy::print_stderr, clippy::print_stdout)]
// ORB-00004: legacy knowledge-graph surfaces still need a focused documentation pass.
#![allow(missing_docs)]
// ORB-00013: Unit tests use unwrap/expect for fixture setup; production call sites remain linted.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_html_tags,
    rustdoc::private_intra_doc_links
)]
//! Knowledge-graph parsing, storage, and query services for Orbit.
//!
//! The scan pipeline applies a built-in `.orbitignore` baseline before files
//! are parsed. By default Orbit excludes `.orbit/`, `node_modules/`, and
//! `target/` from indexing unless a workspace `.orbitignore` file overrides
//! those patterns.

/// Shared default patterns for the scan pipeline and newly initialized
/// workspaces' `.orbitignore` files.
pub const DEFAULT_ORBITIGNORE_PATTERNS: &[&str] = &[
    ".orbit/",
    "node_modules/",
    "target/",
    "dist/",
    "build/",
    ".venv/",
    "venv/",
    "__pycache__/",
    "*.egg-info/",
];

/// Render the default `.orbitignore` file content for `orbit workspace init`.
pub fn default_orbitignore_template() -> String {
    let mut content = String::from(
        "# Common generated/artifact directories that should stay out of the knowledge graph.\n",
    );
    for pattern in DEFAULT_ORBITIGNORE_PATTERNS {
        content.push_str(pattern);
        content.push('\n');
    }
    content
}

pub mod commands;
mod error;
pub mod extract;
pub mod graph;
pub mod graph_bench;
mod io;
pub mod lock;
pub mod metrics;
pub mod pipeline;
mod selector;
pub mod service;
mod store;
pub mod workflows;
pub mod working_graph;

pub use commands::knowledge_error_to_orbit;
pub use error::{KnowledgeError, KnowledgeErrorKind};
pub use graph::GraphReadOptions;
pub use selector::{Selector, SelectorParseError};
pub use service::{TaskGraphScope, TaskGraphService, default_knowledge_dir};
pub use store::{
    DEFAULT_BLOB_CACHE_CAPACITY, DEFAULT_OBJECT_CACHE_CAPACITY, GraphObjectCache,
    KnowledgeEntryKind, KnowledgePack, KnowledgePackAutoRefreshDiagnostic,
    KnowledgePackDiagnostics, KnowledgePackEntry, KnowledgePackErrorDiagnostic,
    KnowledgePackResult, KnowledgePackTimeout, KnowledgeStore, LeafData, SymbolSummary,
    UnresolvedSelectorReason, load_task_working_graph, overlay_pack_with_working_graph,
    pack_from_working_graph, save_task_working_graph, task_working_graph_state_path,
};
pub use working_graph::{
    LeafEdit, LeafVersionChain, MoveResult, WorkingGraph, WorkingLeaf, WriteError, WriteResult,
};

#[cfg(test)]
mod tests;
