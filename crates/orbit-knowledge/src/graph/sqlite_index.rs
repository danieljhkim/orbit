use std::collections::HashMap;
use std::fs;
use std::path::Path;

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, TransactionBehavior, params};

use super::nodes::{BaseNodeFields, CodebaseGraphV1, FileNode, LeafKind};
use crate::error::KnowledgeError;

pub(crate) const GRAPH_SQLITE_INDEX_SCHEMA_VERSION: u32 = 1;
pub(crate) const GRAPH_SQLITE_INDEX_FILENAME: &str = "graph_index.sqlite";

pub(crate) fn write_graph_index(
    path: &Path,
    graph_ref: &str,
    graph: &CodebaseGraphV1,
) -> Result<(), KnowledgeError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "create graph sqlite index dir {}: {error}",
                parent.display()
            ))
        })?;
    }

    let mut conn = open_connection(path)?;
    enable_wal_mode(&conn)?;
    conn.pragma_update(None, "busy_timeout", "5000")
        .map_err(|error| sqlite_error(path, "set busy_timeout", error))?;

    let created_at = previous_created_at_for_same_ref(&conn, graph_ref)?
        .unwrap_or_else(|| Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true));
    let selector_counts = selector_counts(graph);

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| sqlite_error(path, "begin graph sqlite index transaction", error))?;
    tx.execute_batch(
        r#"
        DROP TABLE IF EXISTS meta;
        DROP TABLE IF EXISTS node;
        DROP TABLE IF EXISTS file_summary;

        CREATE TABLE meta (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL
        );

        CREATE TABLE node (
          id TEXT PRIMARY KEY,
          node_type TEXT NOT NULL,
          kind TEXT,
          name TEXT NOT NULL,
          name_lower TEXT NOT NULL,
          location TEXT NOT NULL,
          location_lower TEXT NOT NULL,
          parent_id TEXT,
          selector TEXT
        );
        CREATE INDEX idx_node_name_lower ON node(name_lower);
        CREATE INDEX idx_node_location_lower ON node(location_lower);
        CREATE INDEX idx_node_parent ON node(parent_id);
        CREATE UNIQUE INDEX idx_node_selector ON node(selector) WHERE selector IS NOT NULL;

        CREATE TABLE file_summary (
          file_id TEXT PRIMARY KEY,
          symbol_count INTEGER NOT NULL,
          path TEXT NOT NULL
        );
        CREATE INDEX idx_file_symbol_count ON file_summary(symbol_count DESC);
        "#,
    )
    .map_err(|error| sqlite_error(path, "initialize graph sqlite index schema", error))?;

    tx.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)",
        params![
            "schema_version",
            GRAPH_SQLITE_INDEX_SCHEMA_VERSION.to_string()
        ],
    )
    .map_err(|error| sqlite_error(path, "insert sqlite index schema_version", error))?;
    tx.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)",
        params!["created_at", created_at],
    )
    .map_err(|error| sqlite_error(path, "insert sqlite index created_at", error))?;

    {
        let mut node_insert = tx
            .prepare(
                r#"
                INSERT OR REPLACE INTO node (
                  id, node_type, kind, name, name_lower, location, location_lower, parent_id, selector
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
            )
            .map_err(|error| sqlite_error(path, "prepare graph sqlite node insert", error))?;

        for dir in &graph.dirs {
            let selector = stable_selector(dir_selector(&dir.base), &selector_counts);
            insert_node(
                path,
                &mut node_insert,
                &dir.base,
                "dir",
                None,
                selector.as_deref(),
            )?;
        }

        for file in &graph.files {
            let selector = stable_selector(file_selector(&file.base), &selector_counts);
            insert_node(
                path,
                &mut node_insert,
                &file.base,
                "file",
                None,
                selector.as_deref(),
            )?;
        }

        for leaf in &graph.leaves {
            let kind = leaf.kind.to_string();
            let selector = stable_selector(leaf_selector(&leaf.base, &leaf.kind), &selector_counts);
            insert_node(
                path,
                &mut node_insert,
                &leaf.base,
                "leaf",
                Some(kind.as_str()),
                selector.as_deref(),
            )?;
        }
    }

    {
        let mut file_summary_insert = tx
            .prepare(
                "INSERT OR REPLACE INTO file_summary (file_id, symbol_count, path) VALUES (?1, ?2, ?3)",
            )
            .map_err(|error| {
                sqlite_error(path, "prepare graph sqlite file_summary insert", error)
            })?;

        for file in &graph.files {
            insert_file_summary(path, &mut file_summary_insert, file)?;
        }
    }

    tx.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)",
        params!["graph_ref", graph_ref],
    )
    .map_err(|error| sqlite_error(path, "insert sqlite index graph_ref", error))?;
    tx.commit()
        .map_err(|error| sqlite_error(path, "commit graph sqlite index", error))?;
    Ok(())
}

fn open_connection(path: &Path) -> Result<Connection, KnowledgeError> {
    match Connection::open(path) {
        Ok(conn) => Ok(conn),
        Err(first_error) => {
            if path.exists() {
                fs::remove_file(path).map_err(|remove_error| {
                    KnowledgeError::knowledge_unavailable(format!(
                        "open graph sqlite index {} failed ({first_error}); remove corrupt file failed: {remove_error}",
                        path.display()
                    ))
                })?;
                Connection::open(path)
                    .map_err(|error| sqlite_error(path, "recreate graph sqlite index", error))
            } else {
                Err(sqlite_error(path, "open graph sqlite index", first_error))
            }
        }
    }
}

fn enable_wal_mode(conn: &Connection) -> Result<(), KnowledgeError> {
    let mode = conn
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        .map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "set graph sqlite index journal_mode=WAL: {error}"
            ))
        })?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(KnowledgeError::knowledge_unavailable(format!(
            "set graph sqlite index journal_mode=WAL returned `{mode}`"
        )));
    }
    Ok(())
}

fn previous_created_at_for_same_ref(
    conn: &Connection,
    graph_ref: &str,
) -> Result<Option<String>, KnowledgeError> {
    let expected_schema = GRAPH_SQLITE_INDEX_SCHEMA_VERSION.to_string();
    let schema_version = query_meta(conn, "schema_version")?;
    let previous_graph_ref = query_meta(conn, "graph_ref")?;
    if schema_version.as_deref() == Some(expected_schema.as_str())
        && previous_graph_ref.as_deref() == Some(graph_ref)
    {
        return query_meta(conn, "created_at");
    }
    Ok(None)
}

fn query_meta(conn: &Connection, key: &str) -> Result<Option<String>, KnowledgeError> {
    match conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    ) {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::SqliteFailure(_, _)) => Ok(None),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(KnowledgeError::knowledge_unavailable(format!(
            "read graph sqlite index meta `{key}`: {error}"
        ))),
    }
}

fn insert_node(
    path: &Path,
    statement: &mut rusqlite::Statement<'_>,
    base: &BaseNodeFields,
    node_type: &str,
    kind: Option<&str>,
    selector: Option<&str>,
) -> Result<(), KnowledgeError> {
    statement
        .execute(params![
            base.id.as_str(),
            node_type,
            kind,
            base.name.as_str(),
            base.name.to_lowercase(),
            base.location.as_str(),
            base.location.to_lowercase(),
            base.parent_id.as_deref(),
            selector,
        ])
        .map_err(|error| sqlite_error(path, "insert graph sqlite node", error))?;
    Ok(())
}

fn insert_file_summary(
    path: &Path,
    statement: &mut rusqlite::Statement<'_>,
    file: &FileNode,
) -> Result<(), KnowledgeError> {
    let symbol_count = i64::try_from(file.leaf_children.len()).map_err(|_| {
        KnowledgeError::invalid_data(format!(
            "file `{}` has too many leaf children for sqlite index",
            file.base.id
        ))
    })?;
    statement
        .execute(params![
            file.base.id.as_str(),
            symbol_count,
            file.base.location.as_str()
        ])
        .map_err(|error| sqlite_error(path, "insert graph sqlite file_summary", error))?;
    Ok(())
}

fn selector_counts(graph: &CodebaseGraphV1) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for dir in &graph.dirs {
        count_selector(&mut counts, dir_selector(&dir.base));
    }
    for file in &graph.files {
        count_selector(&mut counts, file_selector(&file.base));
    }
    for leaf in &graph.leaves {
        count_selector(&mut counts, leaf_selector(&leaf.base, &leaf.kind));
    }
    counts
}

fn count_selector(counts: &mut HashMap<String, usize>, selector: String) {
    *counts.entry(selector).or_default() += 1;
}

fn stable_selector(selector: String, counts: &HashMap<String, usize>) -> Option<String> {
    (counts.get(&selector) == Some(&1)).then_some(selector)
}

fn dir_selector(base: &BaseNodeFields) -> String {
    let location = base.location.trim_end_matches('/');
    let location = if location.is_empty() { "." } else { location };
    format!("dir:{location}")
}

fn file_selector(base: &BaseNodeFields) -> String {
    format!("file:{}", base.location)
}

fn leaf_selector(base: &BaseNodeFields, kind: &LeafKind) -> String {
    format!("symbol:{}:{}", base.location, kind)
}

fn sqlite_error(path: &Path, action: &str, error: rusqlite::Error) -> KnowledgeError {
    KnowledgeError::knowledge_unavailable(format!("{action} {}: {error}", path.display()))
}
