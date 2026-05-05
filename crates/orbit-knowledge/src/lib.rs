#![deny(clippy::print_stderr, clippy::print_stdout)]
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

mod error;
pub mod extract;
pub mod graph;
pub mod graph_bench;
mod io;
pub mod lock;
pub mod pipeline;
mod selector;
pub mod service;
mod store;
mod task_id_pattern;
pub mod working_graph;

#[doc(hidden)]
pub mod task_commits {
    //! Sidecar-index re-export so integration tests and downstream crates can
    //! read the task-commits index without reaching into `store` directly.
    pub use crate::store::task_commits::*;
}

pub use error::KnowledgeError;
pub use selector::{Selector, SelectorParseError};
pub use service::history::{
    CommitSummary as HistoryCommitSummary, DEFAULT_STALENESS_THRESHOLD, HistoryQueryOptions,
    HistorySource, StalenessInfo, TaskHistoryEntry, TaskHistoryResult, query_task_history,
};
pub use service::{TaskGraphScope, TaskGraphService, default_knowledge_dir};
pub use store::{
    DEFAULT_BLOB_CACHE_CAPACITY, DEFAULT_OBJECT_CACHE_CAPACITY, GraphObjectCache,
    KnowledgeEntryKind, KnowledgePack, KnowledgePackEntry, KnowledgePackTimeout, KnowledgeStore,
    LeafData, NodeTaskInfo, SymbolSummary, load_task_working_graph,
    overlay_pack_with_working_graph, pack_from_working_graph, save_task_working_graph,
    task_working_graph_state_path,
};
pub use task_id_pattern::{DEFAULT_TASK_ID_PATTERN, ORBIT_TASK_ID_PATTERN, TaskIdPattern};
pub use working_graph::{
    LeafEdit, LeafVersionChain, MoveResult, WorkingGraph, WorkingLeaf, WriteError, WriteResult,
};
