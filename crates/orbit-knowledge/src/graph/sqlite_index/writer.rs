//! Write path: schema creation, WAL, and three-phase ingestion for the graph SQLite index.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, TransactionBehavior, params};

use super::super::nodes::{BaseNodeFields, CodebaseGraphV1, FileNode};
use super::GRAPH_SQLITE_INDEX_SCHEMA_VERSION;
use super::rows::{dir_selector, file_selector, leaf_selector, selector_counts, stable_selector};
use crate::error::KnowledgeError;

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
        DROP TABLE IF EXISTS child;
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
          language TEXT NOT NULL,
          location TEXT NOT NULL,
          location_lower TEXT NOT NULL,
          parent_id TEXT,
          selector TEXT,
          object_hash TEXT NOT NULL,
          ordinal INTEGER NOT NULL,
          scan_order INTEGER NOT NULL
        );
        CREATE INDEX idx_node_name_lower ON node(name_lower);
        CREATE INDEX idx_node_location_lower ON node(location_lower);
        CREATE INDEX idx_node_parent ON node(parent_id);
        CREATE INDEX idx_node_parent_ordinal ON node(parent_id, ordinal);
        CREATE UNIQUE INDEX idx_node_selector ON node(selector) WHERE selector IS NOT NULL;

        -- `child` mirrors the navigator's forward child pointers (DirNode.dir_children
        -- + DirNode.file_children, FileNode.leaf_children, LeafNode.children). The
        -- node.parent_id column is unreliable for nested leaves: build.rs stamps every
        -- leaf with parent_id = file_id even when the leaf's semantic parent is
        -- another leaf (e.g. methods inside a class). T20260510-2 caught this as an
        -- output-equivalence violation between SQL show and the navigator.
        CREATE TABLE child (
          parent_id TEXT NOT NULL,
          child_id TEXT NOT NULL,
          ordinal INTEGER NOT NULL,
          PRIMARY KEY (parent_id, child_id)
        );
        CREATE INDEX idx_child_parent_ordinal ON child(parent_id, ordinal);

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
                  id, node_type, kind, name, name_lower, language, location, location_lower,
                  parent_id, selector, object_hash, ordinal, scan_order
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                "#,
            )
            .map_err(|error| sqlite_error(path, "prepare graph sqlite node insert", error))?;

        let file_scan_offset = graph.dirs.len();
        let leaf_scan_offset = file_scan_offset + graph.files.len();

        for (scan_order, dir) in graph.dirs.iter().enumerate() {
            let selector = stable_selector(dir_selector(&dir.base), &selector_counts);
            let object_hash = object_hash_for(path, node_object_hashes, &dir.base.id)?;
            insert_node(
                path,
                &mut node_insert,
                &dir.base,
                NodeInsertValues {
                    node_type: "dir",
                    kind: None,
                    selector: selector.as_deref(),
                    object_hash,
                    ordinal: child_ordinals.get(&dir.base.id).copied().unwrap_or(0),
                    scan_order: scan_order as i64,
                },
            )?;
        }

        for (index, file) in graph.files.iter().enumerate() {
            let selector = stable_selector(file_selector(&file.base), &selector_counts);
            let object_hash = object_hash_for(path, node_object_hashes, &file.base.id)?;
            insert_node(
                path,
                &mut node_insert,
                &file.base,
                NodeInsertValues {
                    node_type: "file",
                    kind: None,
                    selector: selector.as_deref(),
                    object_hash,
                    ordinal: child_ordinals.get(&file.base.id).copied().unwrap_or(0),
                    scan_order: (file_scan_offset + index) as i64,
                },
            )?;
        }

        for (index, leaf) in graph.leaves.iter().enumerate() {
            let kind = leaf.kind.to_string();
            let selector = stable_selector(leaf_selector(&leaf.base, &leaf.kind), &selector_counts);
            let object_hash = object_hash_for(path, node_object_hashes, &leaf.base.id)?;
            insert_node(
                path,
                &mut node_insert,
                &leaf.base,
                NodeInsertValues {
                    node_type: "leaf",
                    kind: Some(kind.as_str()),
                    selector: selector.as_deref(),
                    object_hash,
                    ordinal: child_ordinals.get(&leaf.base.id).copied().unwrap_or(0),
                    scan_order: (leaf_scan_offset + index) as i64,
                },
            )?;
        }
    }

    {
        let mut child_insert = tx
            .prepare(
                "INSERT OR REPLACE INTO child (parent_id, child_id, ordinal) VALUES (?1, ?2, ?3)",
            )
            .map_err(|error| sqlite_error(path, "prepare graph sqlite child insert", error))?;

        // Forward child pointers, mirroring GraphNodeRef::child_ids():
        //   Dir  -> dir_children + file_children
        //   File -> leaf_children
        //   Leaf -> children (nested leaves)
        for dir in &graph.dirs {
            for (ordinal, child_id) in dir
                .dir_children
                .iter()
                .chain(dir.file_children.iter())
                .enumerate()
            {
                child_insert
                    .execute(params![
                        dir.base.id.as_str(),
                        child_id.as_str(),
                        ordinal as i64
                    ])
                    .map_err(|error| sqlite_error(path, "insert graph sqlite child edge", error))?;
            }
        }
        for file in &graph.files {
            for (ordinal, child_id) in file.leaf_children.iter().enumerate() {
                child_insert
                    .execute(params![
                        file.base.id.as_str(),
                        child_id.as_str(),
                        ordinal as i64
                    ])
                    .map_err(|error| sqlite_error(path, "insert graph sqlite child edge", error))?;
            }
        }
        for leaf in &graph.leaves {
            for (ordinal, child_id) in leaf.children.iter().enumerate() {
                child_insert
                    .execute(params![
                        leaf.base.id.as_str(),
                        child_id.as_str(),
                        ordinal as i64
                    ])
                    .map_err(|error| sqlite_error(path, "insert graph sqlite child edge", error))?;
            }
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

pub(crate) fn previous_created_at_for_same_ref(
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

pub(crate) fn query_meta(conn: &Connection, key: &str) -> Result<Option<String>, KnowledgeError> {
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

pub(crate) fn graph_index_debug_force_fallback() -> bool {
    std::env::var("ORBIT_GRAPH_DEBUG_FORCE_FALLBACK")
        .map(|value| !matches!(value.as_str(), "" | "0" | "false" | "FALSE"))
        .unwrap_or(false)
}

struct NodeInsertValues<'a> {
    node_type: &'a str,
    kind: Option<&'a str>,
    selector: Option<&'a str>,
    object_hash: &'a str,
    ordinal: i64,
    scan_order: i64,
}

fn insert_node(
    path: &Path,
    statement: &mut rusqlite::Statement<'_>,
    base: &BaseNodeFields,
    values: NodeInsertValues<'_>,
) -> Result<(), KnowledgeError> {
    statement
        .execute(params![
            base.id.as_str(),
            values.node_type,
            values.kind,
            base.name.as_str(),
            base.name.to_lowercase(),
            base.language.as_str(),
            base.location.as_str(),
            base.location.to_lowercase(),
            base.parent_id.as_deref(),
            values.selector,
            values.object_hash,
            values.ordinal,
            values.scan_order,
        ])
        .map_err(|error| sqlite_error(path, "insert graph sqlite node", error))?;
    Ok(())
}

pub(crate) fn usize_from_i64(
    path: &Path,
    label: &str,
    value: i64,
) -> Result<usize, KnowledgeError> {
    usize::try_from(value).map_err(|_| {
        KnowledgeError::invalid_data(format!(
            "graph sqlite index {} returned invalid {label} `{value}`",
            path.display()
        ))
    })
}

pub(crate) fn u64_from_i64(path: &Path, label: &str, value: i64) -> Result<u64, KnowledgeError> {
    u64::try_from(value).map_err(|_| {
        KnowledgeError::invalid_data(format!(
            "graph sqlite index {} returned invalid {label} `{value}`",
            path.display()
        ))
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

pub(crate) fn sqlite_error(path: &Path, action: &str, error: rusqlite::Error) -> KnowledgeError {
    KnowledgeError::knowledge_unavailable(format!("{action} {}: {error}", path.display()))
}
