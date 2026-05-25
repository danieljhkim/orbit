//! SQLite-backed graph store and query API skeleton.
//!
//! This crate owns the durable graph database path contract, sync policy, and
//! public query surface. Query and sync behavior lands in later phases; this
//! crate already owns database creation so downstream phases can write against
//! the stable schema.

use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use orbit_graph_extract::Selector;
use rusqlite::Connection;

mod store;
mod sync;

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
    db_path: GraphDbPath,
    worktree_root: PathBuf,
    policy: SyncPolicy,
}

impl Graph {
    /// Open the graph database for `worktree_root` using `policy`.
    pub fn open(worktree_root: &Path, policy: SyncPolicy) -> Result<Self, GraphError> {
        // Phase 4 query methods will call this; keep the dispatcher live under dead-code lints.
        let _ensure_synced: fn(&Self) -> Result<(), GraphError> = Self::ensure_synced;
        let opened = store::open(worktree_root, policy)?;
        Ok(Self {
            db_path: opened.db_path,
            worktree_root: worktree_root.to_path_buf(),
            policy,
        })
    }

    /// Synchronize indexed rows with files on disk.
    pub fn sync(&self, mode: SyncMode) -> Result<SyncReport, GraphError> {
        sync::run(self.db_path.path(), self.worktree_root.as_path(), mode)
    }

    pub(crate) fn ensure_synced(&self) -> Result<(), GraphError> {
        match self.policy {
            SyncPolicy::Manual => Ok(()),
            SyncPolicy::OnRead => self.sync(SyncMode::Auto).map(|_| ()),
            SyncPolicy::Windowed { window } => {
                if sync_window_elapsed(self.last_incremental_at()?, window)? {
                    self.sync(SyncMode::Auto)?;
                }
                Ok(())
            }
        }
    }

    fn last_incremental_at(&self) -> Result<i64, GraphError> {
        let conn = Connection::open(self.db_path.path())
            .map_err(|source| GraphError::sqlite("open graph database for sync policy", source))?;
        let value = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'last_incremental_at'",
                [],
                |row| row.get::<_, String>(0),
            )
            .map_err(|source| {
                GraphError::sqlite("read graph last incremental sync metadata", source)
            })?;
        parse_epoch_nanos("read graph last incremental sync metadata", &value)
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

fn sync_window_elapsed(last_incremental_at: i64, window: Duration) -> Result<bool, GraphError> {
    if last_incremental_at < 0 {
        return Err(GraphError::invalid_data(
            "check graph sync policy window",
            format!("last_incremental_at is negative: {last_incremental_at}"),
        ));
    }
    if last_incremental_at == 0 {
        return Ok(true);
    }

    let now = now_epoch_nanos("check graph sync policy window")?;
    let elapsed = now.saturating_sub(last_incremental_at);
    Ok(u128::try_from(elapsed).map_err(|source| {
        GraphError::invalid_data("check graph sync policy window", source.to_string())
    })? > window.as_nanos())
}

fn parse_epoch_nanos(operation: &'static str, value: &str) -> Result<i64, GraphError> {
    value
        .parse::<i64>()
        .map_err(|source| GraphError::invalid_data(operation, source.to_string()))
}

fn now_epoch_nanos(operation: &'static str) -> Result<i64, GraphError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            GraphError::invalid_data(
                operation,
                format!("system time is before UNIX_EPOCH: {error}"),
            )
        })?;
    i64::try_from(duration.as_nanos())
        .map_err(|error| GraphError::invalid_data(operation, error.to_string()))
}

/// Graph crate error surface.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GraphError {
    /// A filesystem operation failed while opening graph storage.
    Io {
        /// Operation being performed.
        operation: &'static str,
        /// Filesystem path involved in the failed operation.
        path: PathBuf,
        /// Source error rendered as text for cloneable error propagation.
        reason: String,
    },
    /// A SQLite operation failed while opening or initializing graph storage.
    Sqlite {
        /// Operation being performed.
        operation: &'static str,
        /// Source error rendered as text for cloneable error propagation.
        reason: String,
    },
    /// Stored or discovered graph data was invalid.
    InvalidData {
        /// Operation being performed.
        operation: &'static str,
        /// Validation failure rendered as text for cloneable error propagation.
        reason: String,
    },
    /// Placeholder variant until storage, sync, and query errors are defined.
    Unimplemented,
}

impl GraphError {
    pub(crate) fn io(
        operation: &'static str,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            reason: source.to_string(),
        }
    }

    pub(crate) fn sqlite(operation: &'static str, source: rusqlite::Error) -> Self {
        Self::sqlite_message(operation, source.to_string())
    }

    pub(crate) fn sqlite_message(operation: &'static str, reason: impl Into<String>) -> Self {
        Self::Sqlite {
            operation,
            reason: reason.into(),
        }
    }

    pub(crate) fn invalid_data(operation: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidData {
            operation,
            reason: reason.into(),
        }
    }
}

impl Display for GraphError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                reason,
            } => write!(f, "{operation} at {}: {reason}", path.display()),
            Self::Sqlite { operation, reason } => write!(f, "{operation}: {reason}"),
            Self::InvalidData { operation, reason } => write!(f, "{operation}: {reason}"),
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
