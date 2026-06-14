#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

//! SQLite-backed graph store and query API skeleton.
//!
//! This crate owns the durable graph database path contract, sync policy, and
//! public query surface. Query and sync behavior lands in later phases; this
//! crate already owns database creation so downstream phases can write against
//! the stable schema.

use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use git2::Repository;
pub use orbit_graph_extract::Selector;
use rusqlite::{Connection, OpenFlags, params};
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
pub const EXTRACTOR_VERSION: u32 = 4;

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
    read_conn: Mutex<Connection>,
    last_auto_sync_at: Mutex<i64>,
    _watcher: Option<sync::watcher::SyncWatcher>,
}

impl Graph {
    /// Open the graph database for `worktree_root` using `policy`.
    pub fn open(worktree_root: &Path, policy: SyncPolicy) -> Result<Self, GraphError> {
        // Phase 4 query methods will call this; keep the dispatcher live under dead-code lints.
        let _ensure_synced: fn(&Self) -> Result<(), GraphError> = Self::ensure_synced;
        let opened = store::open(worktree_root, policy)?;
        clean_old_databases_excluding(worktree_root, opened.db_path.path())?;
        let read_conn = open_read_connection(opened.db_path.path(), "open graph read connection")?;
        let last_auto_sync_at = read_last_incremental_at(
            &read_conn,
            "read graph last incremental sync metadata at open",
        )?;
        let watcher = if let SyncPolicy::Watch { debounce } = policy {
            Some(sync::watcher::SyncWatcher::start(
                opened.db_path.path().to_path_buf(),
                worktree_root.to_path_buf(),
                debounce,
            )?)
        } else {
            None
        };

        let graph = Self {
            db_path: opened.db_path,
            worktree_root: worktree_root.to_path_buf(),
            policy,
            read_conn: Mutex::new(read_conn),
            last_auto_sync_at: Mutex::new(last_auto_sync_at),
            _watcher: watcher,
        };
        if matches!(policy, SyncPolicy::Watch { .. }) {
            graph.sync(SyncMode::Auto)?;
        }
        Ok(graph)
    }

    /// Synchronize indexed rows with files on disk.
    pub fn sync(&self, mode: SyncMode) -> Result<SyncReport, GraphError> {
        let report = sync::run(self.db_path.path(), self.worktree_root.as_path(), mode)?;
        if mode == SyncMode::Auto {
            self.record_auto_sync_now()?;
        }
        Ok(report)
    }

    /// Return the resolved database path backing this graph handle.
    pub fn db_path(&self) -> &GraphDbPath {
        &self.db_path
    }

    pub(crate) fn ensure_synced(&self) -> Result<(), GraphError> {
        match self.policy {
            SyncPolicy::Manual => Ok(()),
            SyncPolicy::OnRead => self.sync(SyncMode::Auto).map(|_| ()),
            SyncPolicy::Watch { .. } => Ok(()),
            SyncPolicy::Windowed { window } => {
                if sync_window_elapsed(self.last_auto_sync_at(), window)? {
                    self.sync(SyncMode::Auto)?;
                }
                Ok(())
            }
        }
    }

    fn last_auto_sync_at(&self) -> i64 {
        *self
            .last_auto_sync_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn record_auto_sync_now(&self) -> Result<(), GraphError> {
        let now = now_epoch_nanos("record graph auto sync timestamp")?;
        let mut last_auto_sync_at = self
            .last_auto_sync_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *last_auto_sync_at = now;
        Ok(())
    }

    pub(crate) fn with_read_connection<T>(
        &self,
        run: impl FnOnce(&Connection) -> Result<T, GraphError>,
    ) -> Result<T, GraphError> {
        let conn = self
            .read_conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        run(&conn)
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
    pub fn impact(
        &self,
        sel: &Selector,
        depth: u8,
        min_confidence: Confidence,
    ) -> Result<ImpactResult, GraphError> {
        self.ensure_synced()?;
        query::impact::run(self, sel, depth, min_confidence)
    }

    /// Trace the call tree rooted at a command handler.
    pub fn trace(
        &self,
        command: &str,
        depth: u8,
        min_confidence: Confidence,
    ) -> Result<TraceResult, GraphError> {
        self.ensure_synced()?;
        query::trace::run(self, command, depth, min_confidence)
    }

    /// Summarize indexed files and symbols, optionally scoped to a `dir:` or
    /// `file:` selector. Passing `None` summarizes the whole worktree.
    pub fn overview(
        &self,
        scope: Option<&Selector>,
        format: OverviewFormat,
    ) -> Result<OverviewResult, GraphError> {
        self.ensure_synced()?;
        query::overview::run(self, scope, format)
    }

    /// Return the concrete types implementing the trait addressed by `sel`.
    pub fn implementors(&self, sel: &Selector) -> Result<ImplementorsResult, GraphError> {
        self.ensure_synced()?;
        query::implementors::run(self, sel)
    }

    /// Return outbound module/import edges for the files addressed by `sel`
    /// (a `file:` or `dir:` selector).
    pub fn deps(&self, sel: &Selector) -> Result<DepsResult, GraphError> {
        self.ensure_synced()?;
        query::deps::run(self, sel)
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

fn open_read_connection(db_path: &Path, operation: &'static str) -> Result<Connection, GraphError> {
    let conn = Connection::open(db_path).map_err(|source| GraphError::sqlite(operation, source))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|source| GraphError::sqlite("enable foreign keys for graph read", source))?;
    Ok(conn)
}

fn read_last_incremental_at(conn: &Connection, operation: &'static str) -> Result<i64, GraphError> {
    let value = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'last_incremental_at'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map_err(|source| GraphError::sqlite(operation, source))?;
    parse_epoch_nanos(operation, &value)
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
    conn: &Connection,
    sel: &Selector,
) -> Result<Option<SymbolSpan>, GraphError> {
    let Selector::Symbol { path, symbol, kind } = sel else {
        return Ok(None);
    };

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
    /// Run an initial sync at open, then keep the index fresh with a background watcher.
    Watch {
        /// Event coalescing window before a background sync starts.
        debounce: Duration,
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
    resolve_db_path_for_commit(worktree_root, branch, "", extractor_version)
}

/// Resolve the canonical graph database path, using per-commit filenames for detached HEAD.
///
/// Branch-attached graphs keep the branch-scoped filename. Detached HEAD graphs
/// use `detached-<short-sha>.<version>.db` when a commit SHA is available so
/// concurrent detached checkouts on different commits do not churn the same DB.
// ADR-0190: detached HEAD DB filenames include a commit prefix to isolate agents.
pub fn resolve_db_path_for_commit(
    worktree_root: &Path,
    branch: &str,
    commit_sha: &str,
    extractor_version: u32,
) -> GraphDbPath {
    let filename_stem = graph_db_filename_stem(branch, commit_sha);
    let filename = format!("{filename_stem}.{extractor_version}.db");
    GraphDbPath {
        path: worktree_root.join(".orbit").join("graph").join(filename),
        branch: branch.to_string(),
        extractor_version,
    }
}

fn graph_db_filename_stem(branch: &str, commit_sha: &str) -> String {
    if branch == "HEAD" {
        detached_commit_prefix(commit_sha)
            .map(|prefix| format!("detached-{prefix}"))
            .unwrap_or_else(|| sanitize_branch_for_filename(branch))
    } else {
        sanitize_branch_for_filename(branch)
    }
}

fn detached_commit_prefix(commit_sha: &str) -> Option<&str> {
    let commit_sha = commit_sha.trim();
    let prefix = commit_sha.get(..12)?;
    if prefix.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(prefix)
    } else {
        None
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

/// Delete obsolete graph database files.
///
/// Removes old extractor-version files, plus detached-HEAD DBs whose commit is
/// no longer reachable from any local ref.
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
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| {
            GraphError::io("read graph database directory entry", &graph_dir, source)
        })?;
        paths.push(entry.path());
    }
    paths.sort();

    let repo = Repository::discover(worktree_root).ok();
    let mut delete_paths = Vec::new();
    for path in paths {
        if !path.is_file() || is_active_graph_db_family(path.as_path(), active_db_path) {
            continue;
        }
        if should_delete_graph_db_file(path.as_path(), repo.as_ref())? {
            delete_paths.push(path);
        }
    }

    for path in delete_paths {
        fs::remove_file(path.as_path())
            .map_err(|source| GraphError::io("delete old graph database file", &path, source))?;
        deleted.push(path);
    }
    deleted.sort();

    Ok(CleanReport { graph_dir, deleted })
}

fn should_delete_graph_db_file(path: &Path, repo: Option<&Repository>) -> Result<bool, GraphError> {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(false);
    };
    let Some(metadata) = graph_db_file_metadata(file_name) else {
        return Ok(false);
    };

    if metadata.extractor_version != EXTRACTOR_VERSION {
        return Ok(true);
    }

    // ADR-0190: per-commit detached DBs are pruned by Git reachability.
    let Some(commit_prefix) = metadata.detached_commit_prefix else {
        return Ok(false);
    };
    if !detached_db_meta_matches(path, commit_prefix.as_str())? {
        return Ok(false);
    }

    match repo {
        Some(repo) => detached_commit_is_unreachable(repo, commit_prefix.as_str()),
        None => Ok(false),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphDbFileMetadata {
    extractor_version: u32,
    detached_commit_prefix: Option<String>,
}

fn graph_db_file_metadata(file_name: &str) -> Option<GraphDbFileMetadata> {
    let db_name = file_name
        .strip_suffix(".db")
        .or_else(|| file_name.strip_suffix(".db-wal"))
        .or_else(|| file_name.strip_suffix(".db-shm"))
        .or_else(|| file_name.strip_suffix(".db.lock"))?;
    let (stem, version) = db_name.rsplit_once('.')?;
    let extractor_version = version.parse().ok()?;
    let detached_commit_prefix = stem
        .strip_prefix("detached-")
        .filter(|prefix| prefix.len() == 12)
        .filter(|prefix| prefix.chars().all(|ch| ch.is_ascii_hexdigit()))
        .map(str::to_string);
    Some(GraphDbFileMetadata {
        extractor_version,
        detached_commit_prefix,
    })
}

fn is_active_graph_db_family(path: &Path, active_db_path: &Path) -> bool {
    graph_db_base_path(path) == graph_db_base_path(active_db_path)
}

fn graph_db_base_path(path: &Path) -> PathBuf {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.to_path_buf();
    };
    for suffix in [".db-wal", ".db-shm", ".db.lock"] {
        if let Some(stem) = file_name.strip_suffix(suffix) {
            return path.with_file_name(format!("{stem}.db"));
        }
    }
    path.to_path_buf()
}

fn detached_db_meta_matches(path: &Path, commit_prefix: &str) -> Result<bool, GraphError> {
    let db_path = graph_db_base_path(path);
    if !db_path.exists() {
        return Ok(false);
    }
    let Ok(conn) = Connection::open_with_flags(db_path.as_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return Ok(false);
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT key, value FROM meta WHERE key IN ('branch', 'commit_sha')")
    else {
        return Ok(false);
    };
    let mut rows = stmt
        .query([])
        .map_err(|source| GraphError::sqlite("query detached graph metadata", source))?;
    let mut branch = None;
    let mut commit_sha = None;
    while let Some(row) = rows
        .next()
        .map_err(|source| GraphError::sqlite("read detached graph metadata row", source))?
    {
        let key: String = row
            .get(0)
            .map_err(|source| GraphError::sqlite("read detached graph metadata key", source))?;
        let value: String = row
            .get(1)
            .map_err(|source| GraphError::sqlite("read detached graph metadata value", source))?;
        match key.as_str() {
            "branch" => branch = Some(value),
            "commit_sha" => commit_sha = Some(value),
            _ => {}
        }
    }
    Ok(branch.as_deref() == Some("HEAD")
        && commit_sha
            .as_deref()
            .is_some_and(|sha| sha.starts_with(commit_prefix)))
}

fn detached_commit_is_unreachable(
    repo: &Repository,
    commit_prefix: &str,
) -> Result<bool, GraphError> {
    let detached_commit = match repo
        .revparse_single(commit_prefix)
        .and_then(|object| object.peel_to_commit())
    {
        Ok(commit) => commit.id(),
        Err(_) => return Ok(true),
    };

    let refs = repo.references().map_err(|source| {
        GraphError::invalid_data("list git refs for graph DB cleanup", source.to_string())
    })?;
    for reference in refs {
        let reference = reference.map_err(|source| {
            GraphError::invalid_data("read git ref for graph DB cleanup", source.to_string())
        })?;
        let Some(name) = reference.name() else {
            continue;
        };
        if !name.starts_with("refs/") {
            continue;
        }
        let Ok(ref_commit) = reference.peel_to_commit() else {
            continue;
        };
        let reachable = repo
            .graph_descendant_of(ref_commit.id(), detached_commit)
            .map_err(|source| {
                GraphError::invalid_data("check detached graph DB reachability", source.to_string())
            })?;
        if reachable || ref_commit.id() == detached_commit {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Summary returned after removing stale graph database files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CleanReport {
    /// Directory scanned for graph database files.
    pub graph_dir: PathBuf,
    /// Files deleted because their extractor version or detached commit was stale.
    pub deleted: Vec<PathBuf>,
}

/// Reference query options for [`Graph::refs`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefOpts {
    /// Minimum confidence included in returned results.
    pub confidence: RefConfidence,
    /// Optional kind filter for textual refs or structural relations.
    pub kind: Option<RefKind>,
}

/// Confidence floor and output value for graph references.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefConfidence {
    /// Same file, unambiguous match on name and qualified path.
    Exact,
    /// Cross-file reference resolved through an explicit import.
    ImportResolved,
    /// Cross-file reference resolved within the same module namespace.
    #[default]
    SameModule,
    /// Name-only match with ambiguous or weak resolution.
    FuzzyName,
}

/// Confidence floor used by reference and traversal queries.
pub use RefConfidence as Confidence;

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
    /// Lower-confidence references surfaced because the precise floor found no
    /// textual `refs`. Present only when the precise result was empty and a
    /// lower-confidence (`fuzzy_name`) match exists — see [`RefFallback`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<RefFallback>,
}

/// Lower-confidence references surfaced when the precise floor returned no refs.
///
/// Cross-crate call sites routed through `pub use` re-exports resolve only at
/// `fuzzy_name` (name-only) confidence, which the default `same_module` floor
/// excludes. When the precise `refs` list is empty, the query falls back to the
/// fuzzy floor so a genuinely-referenced public API does not look unreferenced.
/// These matches are name-only and may include unrelated symbols sharing the name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefFallback {
    /// Confidence floor used to produce the fallback references (`fuzzy_name`).
    pub confidence: RefConfidence,
    /// Fallback references, each labelled with its own resolution confidence.
    pub refs: Vec<RefEntry>,
    /// Human-readable explanation of why these lower-confidence refs are shown.
    pub note: String,
}

/// Target metadata included in a [`RefResult`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefTarget {
    /// Short symbol name requested or resolved.
    pub name: String,
    /// Fully-qualified symbol name used as the graph query key.
    #[serde(skip_serializing_if = "Option::is_none")]
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
    /// One-based source line containing the call site.
    pub line: usize,
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

/// Output format for [`Graph::overview`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverviewFormat {
    /// Aggregate counts plus the highest-symbol files, without per-file symbols.
    Summary,
    /// Aggregate counts plus every in-scope file and its symbols.
    Full,
}

/// Repository shape summary returned by [`Graph::overview`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OverviewResult {
    /// Format used to build this result.
    pub format: OverviewFormat,
    /// Scope path the summary was restricted to, or `None` for the whole worktree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Number of indexed files in scope.
    pub total_files: usize,
    /// Number of indexed symbols in scope.
    pub total_symbols: usize,
    /// File counts keyed by language.
    pub languages: BTreeMap<String, usize>,
    /// Symbol counts keyed by symbol kind.
    pub symbol_kinds: BTreeMap<String, usize>,
    /// Files in scope. In `summary` format these are the top files by symbol
    /// count with empty `symbols`; in `full` format every in-scope file with
    /// its symbols.
    pub files: Vec<OverviewFile>,
}

/// A file entry in an [`OverviewResult`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OverviewFile {
    /// Worktree-relative file path.
    pub path: String,
    /// Detected language.
    pub lang: String,
    /// Number of symbols defined in this file.
    pub symbol_count: usize,
    /// Symbols defined in this file; populated only in `full` format.
    pub symbols: Vec<OverviewSymbol>,
}

/// A symbol entry in an [`OverviewFile`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OverviewSymbol {
    /// Short symbol name.
    pub name: String,
    /// Symbol kind.
    pub kind: String,
    /// Fully-qualified symbol name.
    pub qualified: String,
}

/// Trait implementor result returned by [`Graph::implementors`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImplementorsResult {
    /// Trait name matched against, derived from the selector's trailing segment.
    pub trait_name: String,
    /// Concrete types implementing the trait.
    pub implementors: Vec<Implementor>,
}

/// A single trait implementor entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Implementor {
    /// Qualified name of the implementing type.
    pub type_qualified: String,
    /// Trait reference recorded at the impl site (`relations.to_qualified`).
    pub trait_matched: String,
    /// Structural relation kind (`impl` / `implements`).
    pub kind: RefKind,
    /// File defining the implementation.
    pub file: String,
}

/// Outbound module/import edge result returned by [`Graph::deps`].
///
/// Reports source-level import edges, not the Cargo crate dependency graph that
/// v1 `orbit.graph.deps` returned. See [`query::deps`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DepsResult {
    /// Selector scope echoed back.
    pub scope: String,
    /// Outbound import edges in scope.
    pub imports: Vec<DepEdge>,
}

/// A single outbound import edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DepEdge {
    /// Source file that declares the import.
    pub from_file: String,
    /// Imported module path or specifier (language-specific opaque string).
    pub target_path: String,
    /// Imported symbol, or `None` for a whole-module import.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_symbol: Option<String>,
}
