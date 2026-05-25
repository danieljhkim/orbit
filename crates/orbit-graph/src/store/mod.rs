//! SQLite connection setup for the graph store.

pub(crate) mod schema;

use std::fs;
use std::path::Path;

use git2::Repository;
use rusqlite::Connection;

use crate::{EXTRACTOR_VERSION, GraphDbPath, GraphError, SyncPolicy, resolve_db_path};

pub(crate) struct OpenedGraph {
    pub(crate) conn: Connection,
    pub(crate) db_path: GraphDbPath,
}

pub(crate) fn open(worktree_root: &Path, _policy: SyncPolicy) -> Result<OpenedGraph, GraphError> {
    let git = GitContext::for_worktree(worktree_root);
    let db_path = resolve_db_path(worktree_root, git.branch.as_str(), EXTRACTOR_VERSION);

    if let Some(parent) = db_path.path().parent() {
        fs::create_dir_all(parent)
            .map_err(|source| GraphError::io("create graph database directory", parent, source))?;
    }

    let mut conn = Connection::open(db_path.path())
        .map_err(|source| GraphError::sqlite("open graph database", source))?;
    configure_connection(&conn)?;

    if schema::database_is_empty(&conn)? {
        schema::initialize(
            &mut conn,
            &schema::InitialMeta {
                extractor_version: EXTRACTOR_VERSION,
                branch: git.branch.as_str(),
                commit_sha: git.commit_sha.as_str(),
            },
        )?;
    }

    Ok(OpenedGraph { conn, db_path })
}

fn configure_connection(conn: &Connection) -> Result<(), GraphError> {
    let journal_mode = conn
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        .map_err(|source| GraphError::sqlite("set journal_mode=WAL", source))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(GraphError::sqlite_message(
            "set journal_mode=WAL",
            format!("SQLite kept journal_mode={journal_mode}"),
        ));
    }

    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|source| GraphError::sqlite("set foreign_keys=ON", source))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|source| GraphError::sqlite("set synchronous=NORMAL", source))?;
    Ok(())
}

struct GitContext {
    branch: String,
    commit_sha: String,
}

impl GitContext {
    fn for_worktree(worktree_root: &Path) -> Self {
        let Ok(repo) = Repository::discover(worktree_root) else {
            return Self::without_git();
        };
        let Ok(head) = repo.head() else {
            return Self::without_git();
        };

        let branch = if head.is_branch() {
            head.shorthand()
                .filter(|name| !name.is_empty())
                .unwrap_or("HEAD")
                .to_string()
        } else {
            "HEAD".to_string()
        };
        let commit_sha = if head.is_branch() {
            head.target().map(|oid| oid.to_string()).unwrap_or_default()
        } else {
            String::new()
        };

        Self { branch, commit_sha }
    }

    fn without_git() -> Self {
        Self {
            branch: "HEAD".to_string(),
            commit_sha: String::new(),
        }
    }
}

#[cfg(test)]
mod tests;
