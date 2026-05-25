//! SQLite-backed graph store and query API skeleton.
//!
//! This crate owns the durable graph database path contract, sync policy, and
//! public query surface. Storage, sync, and query behavior lands in later
//! phases; this task establishes the type contract those phases implement.

use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use orbit_graph_extract::Selector;

#[cfg(test)]
mod tests;

/// Extractor/storage compatibility version embedded in graph database names.
///
/// Bump this when extractor output or storage expectations change
/// incompatibly. Older graph DB files then become invisible to the active
/// graph handle and are removed by the next sync.
pub const EXTRACTOR_VERSION: u32 = 1;

/// Opaque handle to a worktree-scoped graph database.
pub struct Graph {
    _private: (),
}

impl Graph {
    /// Open the graph database for `worktree_root` using `policy`.
    pub fn open(worktree_root: &Path, policy: SyncPolicy) -> Result<Self, GraphError> {
        let _ = (worktree_root, policy);
        todo!("initialize graph storage")
    }

    /// Synchronize indexed rows with files on disk.
    pub fn sync(&self, mode: SyncMode) -> Result<SyncReport, GraphError> {
        let _ = (self, mode);
        todo!("sync graph rows")
    }

    /// Search indexed symbols, strings, and config keys.
    pub fn search(&self, q: &SearchQuery) -> Result<Vec<Match>, GraphError> {
        let _ = (self, q);
        todo!("search graph index")
    }

    /// Show the node or source view addressed by `sel`.
    pub fn show(&self, sel: &Selector) -> Result<Option<NodeView>, GraphError> {
        let _ = (self, sel);
        todo!("show graph node")
    }

    /// Return inbound references and relations for `sel`.
    pub fn refs(&self, sel: &Selector, opts: &RefOpts) -> Result<RefResult, GraphError> {
        let _ = (self, sel, opts);
        todo!("query graph refs")
    }

    /// Return outbound call edges from `sel`.
    pub fn callees(&self, sel: &Selector) -> Result<Vec<CalleeEdge>, GraphError> {
        let _ = (self, sel);
        todo!("query graph callees")
    }

    /// Return the bounded impact set around `sel`.
    pub fn impact(&self, sel: &Selector, depth: u8) -> Result<ImpactResult, GraphError> {
        let _ = (self, sel, depth);
        todo!("query graph impact")
    }

    /// Trace the call tree rooted at a command handler.
    pub fn trace(&self, command: &str, depth: u8) -> Result<TraceResult, GraphError> {
        let _ = (self, command, depth);
        todo!("trace graph command")
    }
}

/// Graph crate error surface.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GraphError {
    /// Placeholder variant until storage, sync, and query errors are defined.
    Unimplemented,
}

impl Display for GraphError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unimplemented => f.write_str("graph operation is not implemented"),
        }
    }
}

impl std::error::Error for GraphError {}

/// Sync mode requested by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// Incremental sync driven by file metadata and content hashes.
    Auto,
    /// Full sync that rehashes and re-extracts all indexable files.
    Full,
}

/// Policy controlling whether reads refresh the graph before querying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPolicy {
    /// Never auto-sync; callers invoke [`Graph::sync`] explicitly.
    Manual,
    /// Sync inline on every query.
    OnRead,
    /// Sync inline only if the last successful sync is older than `window`.
    Windowed {
        /// Maximum age of the last successful sync before reads refresh.
        window: Duration,
    },
}

/// Summary returned after a graph sync completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    /// Number of files present in the graph after sync.
    pub files_indexed: usize,
    /// Number of files inserted or refreshed by this sync.
    pub files_changed: usize,
    /// Number of files removed from the graph by this sync.
    pub files_removed: usize,
    /// Wall-clock duration spent syncing.
    pub duration: Duration,
}

/// Resolved, worktree-scoped graph database path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphDbPath {
    path: PathBuf,
    branch: String,
    extractor_version: u32,
}

impl GraphDbPath {
    /// Return the canonical SQLite database path.
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    /// Return the unsanitized branch name represented by this database path.
    pub fn branch(&self) -> &str {
        self.branch.as_str()
    }

    /// Return the extractor version embedded in the database filename.
    pub fn extractor_version(&self) -> u32 {
        self.extractor_version
    }
}

/// Resolve the canonical graph database path for a worktree and branch.
///
/// The filename sanitizes the branch by replacing `/` with `_`, while the
/// returned [`GraphDbPath`] keeps the raw branch name for future `meta.branch`
/// storage.
pub fn resolve_db_path(worktree_root: &Path, branch: &str, extractor_version: u32) -> GraphDbPath {
    let sanitized_branch = branch.replace('/', "_");
    let filename = format!("{sanitized_branch}.{extractor_version}.db");
    GraphDbPath {
        path: worktree_root.join(".orbit").join("graph").join(filename),
        branch: branch.to_string(),
        extractor_version,
    }
}

/// Search request for the graph query surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery;

/// Search match returned by [`Graph::search`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match;

/// Source and metadata view returned by [`Graph::show`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeView;

/// Reference query options for [`Graph::refs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefOpts;

/// Reference query result returned by [`Graph::refs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefResult;

/// Outbound call edge returned by [`Graph::callees`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalleeEdge;

/// Bounded impact result returned by [`Graph::impact`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactResult;

/// Command trace result returned by [`Graph::trace`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceResult;
