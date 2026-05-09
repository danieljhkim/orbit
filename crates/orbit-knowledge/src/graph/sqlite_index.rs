use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, TransactionBehavior, params};

use super::nodes::{BaseNodeFields, CodebaseGraphV1, FileNode, LeafKind};
use crate::error::KnowledgeError;

pub(crate) const GRAPH_SQLITE_INDEX_SCHEMA_VERSION: u32 = 2;
pub(crate) const GRAPH_SQLITE_INDEX_FILENAME: &str = "graph_index.sqlite";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphIndexNodeRow {
    pub id: String,
    pub node_type: String,
    pub kind: Option<String>,
    pub location: String,
    pub parent_id: Option<String>,
    pub selector: Option<String>,
    pub object_hash: String,
}

pub struct GraphIndexReader {
    conn: Connection,
    path: PathBuf,
}

impl GraphIndexReader {
    pub fn open_current(
        path: impl AsRef<Path>,
        graph_ref: &str,
    ) -> Result<Option<Self>, KnowledgeError> {
        let path = path.as_ref();
        if !path.is_file() {
            return Ok(None);
        }

        let conn = match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(conn) => conn,
            Err(_) => return Ok(None),
        };
        conn.pragma_update(None, "busy_timeout", "5000")
            .map_err(|error| sqlite_error(path, "set busy_timeout", error))?;

        let expected_schema = GRAPH_SQLITE_INDEX_SCHEMA_VERSION.to_string();
        if query_meta(&conn, "schema_version")?.as_deref() != Some(expected_schema.as_str()) {
            return Ok(None);
        }
        if query_meta(&conn, "graph_ref")?.as_deref() != Some(graph_ref) {
            return Ok(None);
        }

        Ok(Some(Self {
            conn,
            path: path.to_path_buf(),
        }))
    }

    pub fn find_node_by_selector(
        &self,
        selector: &str,
    ) -> Result<Option<GraphIndexNodeRow>, KnowledgeError> {
        self.conn
            .query_row(
                r#"
                SELECT id, node_type, kind, location, parent_id, selector, object_hash
                FROM node
                WHERE selector = ?1
                LIMIT 1
                "#,
                params![selector],
                graph_index_node_from_row,
            )
            .optional()
            .map_err(|error| sqlite_error(&self.path, "query graph sqlite selector", error))
    }

    pub fn node_by_id(&self, id: &str) -> Result<Option<GraphIndexNodeRow>, KnowledgeError> {
        self.conn
            .query_row(
                r#"
                SELECT id, node_type, kind, location, parent_id, selector, object_hash
                FROM node
                WHERE id = ?1
                LIMIT 1
                "#,
                params![id],
                graph_index_node_from_row,
            )
            .optional()
            .map_err(|error| sqlite_error(&self.path, "query graph sqlite node by id", error))
    }

    pub fn children_of(
        &self,
        parent_id: &str,
        limit: usize,
    ) -> Result<Vec<GraphIndexNodeRow>, KnowledgeError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let limit = i64::try_from(limit).map_err(|_| {
            KnowledgeError::invalid_data("graph sqlite child limit exceeds i64".to_string())
        })?;
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT id, node_type, kind, location, parent_id, selector, object_hash
                FROM node
                WHERE parent_id = ?1
                ORDER BY ordinal ASC, id ASC
                LIMIT ?2
                "#,
            )
            .map_err(|error| {
                sqlite_error(&self.path, "prepare graph sqlite children query", error)
            })?;
        let rows = stmt
            .query_map(params![parent_id, limit], graph_index_node_from_row)
            .map_err(|error| sqlite_error(&self.path, "query graph sqlite children", error))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| sqlite_error(&self.path, "read graph sqlite child row", error))
    }

    pub fn lineage_for(
        &self,
        parent_id: Option<&str>,
        depth: usize,
    ) -> Result<Vec<GraphIndexNodeRow>, KnowledgeError> {
        if depth == 0 {
            return Ok(Vec::new());
        }

        let mut lineage = Vec::new();
        let mut next_id = parent_id.map(ToOwned::to_owned);
        while let Some(id) = next_id {
            let row = self.node_by_id(&id)?.ok_or_else(|| {
                KnowledgeError::invalid_data(format!(
                    "graph sqlite index references missing parent node `{id}`"
                ))
            })?;
            next_id = row.parent_id.clone();
            lineage.push(row);
        }
        lineage.reverse();

        if lineage.len() > depth {
            Ok(lineage.split_off(lineage.len() - depth))
        } else {
            Ok(lineage)
        }
    }
}

pub(crate) fn write_graph_index(
    path: &Path,
    graph_ref: &str,
    graph: &CodebaseGraphV1,
    node_object_hashes: &HashMap<String, String>,
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
          selector TEXT,
          object_hash TEXT NOT NULL,
          ordinal INTEGER NOT NULL
        );
        CREATE INDEX idx_node_name_lower ON node(name_lower);
        CREATE INDEX idx_node_location_lower ON node(location_lower);
        CREATE INDEX idx_node_parent ON node(parent_id);
        CREATE INDEX idx_node_parent_ordinal ON node(parent_id, ordinal);
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
        let child_ordinals = child_ordinals(graph);
        let mut node_insert = tx
            .prepare(
                r#"
                INSERT OR REPLACE INTO node (
                  id, node_type, kind, name, name_lower, location, location_lower, parent_id,
                  selector, object_hash, ordinal
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                "#,
            )
            .map_err(|error| sqlite_error(path, "prepare graph sqlite node insert", error))?;

        for dir in &graph.dirs {
            let selector = stable_selector(dir_selector(&dir.base), &selector_counts);
            let object_hash = object_hash_for(path, node_object_hashes, &dir.base.id)?;
            insert_node(
                path,
                &mut node_insert,
                &dir.base,
                "dir",
                None,
                selector.as_deref(),
                object_hash,
                child_ordinals.get(&dir.base.id).copied().unwrap_or(0),
            )?;
        }

        for file in &graph.files {
            let selector = stable_selector(file_selector(&file.base), &selector_counts);
            let object_hash = object_hash_for(path, node_object_hashes, &file.base.id)?;
            insert_node(
                path,
                &mut node_insert,
                &file.base,
                "file",
                None,
                selector.as_deref(),
                object_hash,
                child_ordinals.get(&file.base.id).copied().unwrap_or(0),
            )?;
        }

        for leaf in &graph.leaves {
            let kind = leaf.kind.to_string();
            let selector = stable_selector(leaf_selector(&leaf.base, &leaf.kind), &selector_counts);
            let object_hash = object_hash_for(path, node_object_hashes, &leaf.base.id)?;
            insert_node(
                path,
                &mut node_insert,
                &leaf.base,
                "leaf",
                Some(kind.as_str()),
                selector.as_deref(),
                object_hash,
                child_ordinals.get(&leaf.base.id).copied().unwrap_or(0),
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
    object_hash: &str,
    ordinal: i64,
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
            object_hash,
            ordinal,
        ])
        .map_err(|error| sqlite_error(path, "insert graph sqlite node", error))?;
    Ok(())
}

fn graph_index_node_from_row(row: &Row<'_>) -> rusqlite::Result<GraphIndexNodeRow> {
    Ok(GraphIndexNodeRow {
        id: row.get(0)?,
        node_type: row.get(1)?,
        kind: row.get(2)?,
        location: row.get(3)?,
        parent_id: row.get(4)?,
        selector: row.get(5)?,
        object_hash: row.get(6)?,
    })
}

fn object_hash_for<'a>(
    path: &Path,
    node_object_hashes: &'a HashMap<String, String>,
    node_id: &str,
) -> Result<&'a str, KnowledgeError> {
    node_object_hashes
        .get(node_id)
        .map(String::as_str)
        .ok_or_else(|| {
            KnowledgeError::invalid_data(format!(
                "graph sqlite index {} missing object hash for node `{node_id}`",
                path.display()
            ))
        })
}

fn child_ordinals(graph: &CodebaseGraphV1) -> HashMap<String, i64> {
    let mut ordinals = HashMap::new();
    for dir in &graph.dirs {
        for (ordinal, child_id) in dir
            .dir_children
            .iter()
            .chain(dir.file_children.iter())
            .enumerate()
        {
            ordinals.insert(child_id.clone(), ordinal as i64);
        }
    }
    for file in &graph.files {
        for (ordinal, child_id) in file.leaf_children.iter().enumerate() {
            ordinals.insert(child_id.clone(), ordinal as i64);
        }
    }
    for leaf in &graph.leaves {
        for (ordinal, child_id) in leaf.children.iter().enumerate() {
            ordinals.insert(child_id.clone(), ordinal as i64);
        }
    }
    ordinals
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
