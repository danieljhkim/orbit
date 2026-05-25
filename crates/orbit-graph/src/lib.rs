#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

//! SQLite-backed graph store and query API skeleton.
//!
//! This crate owns the durable graph database path contract, sync policy, and
//! public query surface. Query and sync behavior lands in later phases; this
//! crate already owns database creation so downstream phases can write against
//! the stable schema.

use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use orbit_graph_extract::Selector;
use rusqlite::{Connection, params};
use serde::Serialize;

mod query;
mod store;
mod sync;

pub use query::{
    DEFAULT_SEARCH_LIMIT, DEFAULT_SHOW_MAX_BYTES, Match, NodeMetadata, NodeView, SearchKind,
    SearchQuery, SearchResult, SourceSpan,
};

#[cfg(test)]
mod tests;

/// Extractor/storage compatibility version embedded in graph database names.
///
/// Bump this when extractor output or storage expectations change
/// incompatibly. Older graph DB files then become invisible to the active
/// graph handle and are removed by the next sync.
// L-0052: FTS population invariants require a fresh DB when old indexes may be empty.
pub const EXTRACTOR_VERSION: u32 = 3;

/// Default graph distance used by callers that do not supply `--depth`.
pub const DEFAULT_IMPACT_DEPTH: u8 = 3;

/// Default call-tree distance used by command traces when depth is omitted.
pub const DEFAULT_TRACE_DEPTH: u8 = 5;

/// Maximum number of impacted symbols returned by bounded traversals.
pub const IMPACT_NODE_CAP: usize = 200;

/// Maximum number of trace nodes returned by command traces.
pub const TRACE_NODE_CAP: usize = 200;

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
        clean_old_databases_excluding(worktree_root, opened.db_path.path())?;
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

    /// Return the resolved database path backing this graph handle.
    pub fn db_path(&self) -> &GraphDbPath {
        &self.db_path
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
    pub fn search(&self, q: &SearchQuery) -> Result<SearchResult, GraphError> {
        self.ensure_synced()?;
        query::search::run(self, q)
    }

    /// Show the node or source view addressed by `sel`.
    ///
    /// `max_bytes` bounds the returned source slice. [`DEFAULT_SHOW_MAX_BYTES`]
    /// is the intended CLI/MCP default.
    pub fn show(&self, sel: &Selector, max_bytes: usize) -> Result<Option<NodeView>, GraphError> {
        self.ensure_synced()?;
        query::show::run(self, sel, max_bytes)
    }

    /// Return inbound references and relations for `sel`.
    pub fn refs(&self, sel: &Selector, opts: &RefOpts) -> Result<RefResult, GraphError> {
        self.ensure_synced()?;
        query::refs::run(self, sel, opts)
    }

    /// Return outbound call edges from `sel`.
    pub fn callees(&self, sel: &Selector) -> Result<Vec<CalleeEdge>, GraphError> {
        self.ensure_synced()?;
        query::callees::run(self, sel)
    }

    /// Return the bounded impact set around `sel`.
    pub fn impact(&self, sel: &Selector, depth: u8) -> Result<ImpactResult, GraphError> {
        self.ensure_synced()?;
        query::impact::run(self, sel, depth)
    }

    /// Trace the call tree rooted at a command handler.
    pub fn trace(&self, command: &str, depth: u8) -> Result<TraceResult, GraphError> {
        self.ensure_synced()?;
        query::trace::run(self, command, depth)
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

/// Internal result of resolving a Selector to a symbol's file and span for
/// span-containment queries like callees.
#[derive(Debug, Clone)]
pub(crate) struct SymbolSpan {
    pub(crate) file_path: String,
    pub(crate) span_start: i64,
    pub(crate) span_end: i64,
}

/// Resolve a Selector to a single symbol's (file_path, span) if it exists in
/// the graph. Returns None for selectors that do not map to a stored symbol
/// (including non-Symbol variants and unknown names). Used by read queries
/// that then perform containment or adjacency lookups.
pub(crate) fn resolve_symbol_span(
    db_path: &Path,
    sel: &Selector,
) -> Result<Option<SymbolSpan>, GraphError> {
    let Selector::Symbol { path, symbol, kind } = sel else {
        return Ok(None);
    };

    let conn = Connection::open(db_path)
        .map_err(|source| GraphError::sqlite("open graph database for symbol resolve", source))?;

    // Match on either short name or qualified; apply kind filter when provided.
    // Paths in DB are normalized (slash-separated, relative to worktree).
    let mut sql = String::from(
        "SELECT file_path, span_start, span_end FROM symbols
         WHERE file_path = ?1 AND (name = ?2 OR qualified = ?2)",
    );
    let has_kind = !kind.trim().is_empty();
    if has_kind {
        sql.push_str(" AND kind = ?3");
    }
    sql.push_str(" ORDER BY id LIMIT 1");

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|source| GraphError::sqlite("prepare symbol resolve for query", source))?;

    let row = if has_kind {
        stmt.query_row(params![path, symbol, kind.trim()], |r| {
            Ok(SymbolSpan {
                file_path: r.get(0)?,
                span_start: r.get(1)?,
                span_end: r.get(2)?,
            })
        })
    } else {
        stmt.query_row(params![path, symbol], |r| {
            Ok(SymbolSpan {
                file_path: r.get(0)?,
                span_start: r.get(1)?,
                span_end: r.get(2)?,
            })
        })
    };

    match row {
        Ok(s) => Ok(Some(s)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(source) => Err(GraphError::sqlite("resolve symbol for query", source)),
    }
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
/// The filename sanitizes the branch with a conservative filesystem-safe
/// allowlist, while the returned [`GraphDbPath`] keeps the raw branch name for
/// future `meta.branch` storage.
pub fn resolve_db_path(worktree_root: &Path, branch: &str, extractor_version: u32) -> GraphDbPath {
    let sanitized_branch = sanitize_branch_for_filename(branch);
    let filename = format!("{sanitized_branch}.{extractor_version}.db");
    GraphDbPath {
        path: worktree_root.join(".orbit").join("graph").join(filename),
        branch: branch.to_string(),
        extractor_version,
    }
}

fn sanitize_branch_for_filename(branch: &str) -> String {
    if branch.is_empty() {
        return "_".to_string();
    }

    let chars = branch.chars().collect::<Vec<_>>();
    let mut sanitized = String::with_capacity(branch.len());
    for (index, ch) in chars.iter().copied().enumerate() {
        let is_dot = ch == '.';
        let is_double_dot = is_dot
            && ((index > 0 && chars[index - 1] == '.')
                || (index + 1 < chars.len() && chars[index + 1] == '.'));
        let allowed = ch.is_ascii_alphanumeric()
            || ch == '_'
            || ch == '-'
            || (index > 0 && is_dot && !is_double_dot);

        sanitized.push(if allowed { ch } else { '_' });
    }
    sanitized
}

/// Delete graph database files whose filename embeds an old extractor version.
pub fn clean_old_databases(worktree_root: &Path) -> Result<CleanReport, GraphError> {
    let opened = store::open(worktree_root, SyncPolicy::Manual)?;
    clean_old_databases_excluding(worktree_root, opened.db_path.path())
}

fn clean_old_databases_excluding(
    worktree_root: &Path,
    active_db_path: &Path,
) -> Result<CleanReport, GraphError> {
    let graph_dir = active_db_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| worktree_root.join(".orbit").join("graph"));
    let mut deleted = Vec::new();

    if !graph_dir.exists() {
        return Ok(CleanReport { graph_dir, deleted });
    }

    let entries = fs::read_dir(graph_dir.as_path())
        .map_err(|source| GraphError::io("read graph database directory", &graph_dir, source))?;
    for entry in entries {
        let entry = entry.map_err(|source| {
            GraphError::io("read graph database directory entry", &graph_dir, source)
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if graph_db_file_extractor_version(file_name)
            .is_some_and(|version| version != EXTRACTOR_VERSION)
            && path != active_db_path
        {
            fs::remove_file(path.as_path()).map_err(|source| {
                GraphError::io("delete old graph database file", &path, source)
            })?;
            deleted.push(path);
        }
    }
    deleted.sort();

    Ok(CleanReport { graph_dir, deleted })
}

fn graph_db_file_extractor_version(file_name: &str) -> Option<u32> {
    let db_name = file_name
        .strip_suffix(".db")
        .or_else(|| file_name.strip_suffix(".db-wal"))
        .or_else(|| file_name.strip_suffix(".db-shm"))
        .or_else(|| file_name.strip_suffix(".db.lock"))?;
    db_name.rsplit_once('.')?.1.parse().ok()
}

/// Summary returned after removing stale graph database files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CleanReport {
    /// Directory scanned for graph database files.
    pub graph_dir: PathBuf,
    /// Files deleted because their extractor version was not current.
    pub deleted: Vec<PathBuf>,
}

/// Reference query options for [`Graph::refs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefOpts {
    /// Minimum confidence included in returned results.
    pub confidence: RefConfidence,
    /// Optional kind filter for textual refs or structural relations.
    pub kind: Option<RefKind>,
}

impl Default for RefOpts {
    fn default() -> Self {
        Self {
            confidence: RefConfidence::SameModule,
            kind: None,
        }
    }
}

/// Confidence floor and output value for graph references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefConfidence {
    /// Same file, unambiguous match on name and qualified path.
    Exact,
    /// Cross-file reference resolved through an explicit import.
    ImportResolved,
    /// Cross-file reference resolved within the same module namespace.
    SameModule,
    /// Name-only match with ambiguous or weak resolution.
    FuzzyName,
}

/// Filterable reference and relation kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// Function or method call reference.
    Call,
    /// Type usage reference.
    Type,
    /// Import or use-statement reference.
    Use,
    /// Trait-bound reference.
    TraitBound,
    /// Implementation relation.
    Impl,
    /// Inheritance relation.
    Extends,
    /// Interface implementation relation.
    Implements,
}

/// Reference query result returned by [`Graph::refs`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefResult {
    /// Resolved target symbol, or the unresolved selector target.
    pub target: RefTarget,
    /// Textual references anchored to a source file and span.
    pub refs: Vec<RefEntry>,
    /// Structural relations whose destination is the target symbol.
    pub relations: Vec<RelationEntry>,
    /// Number of candidate rows excluded by the confidence floor.
    pub skipped_low_confidence: usize,
}

/// Target metadata included in a [`RefResult`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefTarget {
    /// Short symbol name requested or resolved.
    pub name: String,
    /// Fully-qualified symbol name used as the graph query key.
    pub qualified: Option<String>,
}

/// Textual reference entry returned by [`Graph::refs`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefEntry {
    /// Source file containing the reference.
    pub file: String,
    /// One-based source line containing the reference.
    pub line: usize,
    /// Textual reference kind.
    pub kind: RefKind,
    /// Resolution confidence for this reference.
    pub confidence: RefConfidence,
}

/// Structural relation entry returned by [`Graph::refs`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelationEntry {
    /// Qualified source symbol for the relation.
    pub from: String,
    /// Structural relation kind.
    pub kind: RefKind,
    /// Source file defining the relation.
    pub file: String,
    /// One-based source line defining the relation.
    pub line: usize,
    /// Resolution confidence for this relation.
    pub confidence: RefConfidence,
}

/// Outbound call edge returned by [`Graph::callees`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CalleeEdge {
    /// Short target name as written in the call site.
    pub target_name: String,
    /// Resolved qualified name (authoritative when present); None for fuzzy/unresolved.
    pub target_qualified: Option<String>,
    /// Confidence label emitted verbatim by the resolver (P3.3) at write time.
    pub confidence: String,
    /// Start byte offset of the call site span inside its source file (minimum for attribution).
    pub from_span: i64,
}

/// Bounded impact result returned by [`Graph::impact`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImpactResult {
    /// Impacted symbols in breadth-first order from the origin.
    pub touched: Vec<ImpactEntry>,
    /// Whether traversal stopped because [`IMPACT_NODE_CAP`] was reached.
    pub truncated: bool,
    /// Number of impacted symbols returned in `touched`.
    pub visited_nodes: usize,
}

/// A symbol reached by [`Graph::impact`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImpactEntry {
    /// Qualified symbol name reached by the traversal.
    pub qualified_name: String,
    /// Breadth-first distance from the origin symbol.
    pub distance: usize,
    /// Edge kind used for the prior hop into this symbol.
    pub edge_kind: RefKind,
}

/// Command trace result returned by [`Graph::trace`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceResult {
    /// Root command handler node, or `None` when the command is unknown.
    pub root: Option<TraceNode>,
    /// Whether traversal stopped because [`TRACE_NODE_CAP`] was reached.
    pub truncated: bool,
    /// Number of nodes returned in the trace tree, including the root.
    pub visited_nodes: usize,
}

impl TraceResult {
    pub(crate) fn empty() -> Self {
        Self {
            root: None,
            truncated: false,
            visited_nodes: 0,
        }
    }
}

/// A node in a command-handler call tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceNode {
    /// Short name as written at the call site, or the handler symbol name for the root.
    pub name: String,
    /// Resolved qualified symbol name when the call target was resolved.
    pub qualified_name: Option<String>,
    /// Resolver confidence for the edge into this node; `None` for the root.
    pub confidence: Option<String>,
    /// Nested callees reached from this symbol.
    pub children: Vec<TraceNode>,
}
