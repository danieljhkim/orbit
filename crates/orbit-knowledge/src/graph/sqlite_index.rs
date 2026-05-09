//! Read-only access to the SQLite graph secondary index.
//!
//! `GraphIndexReader::open` returns `Ok(None)` when the index is absent or
//! stale for the requested graph ref/schema version. That is the
//! graceful-fallback signal: callers must use the existing object-store graph
//! read path when they receive `None`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, TransactionBehavior, params};

use super::nodes::{BaseNodeFields, CodebaseGraphV1, FileNode, LeafKind};
use crate::error::KnowledgeError;

pub(crate) const GRAPH_SQLITE_INDEX_SCHEMA_VERSION: u32 = 4;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphIndexSearchRow {
    pub id: String,
    pub node_type: String,
    pub kind: Option<String>,
    pub name: String,
    pub location: String,
    pub selector: Option<String>,
    pub scan_order: i64,
}

pub struct GraphIndexReader {
    conn: Connection,
    path: PathBuf,
}

impl GraphIndexReader {
    pub fn open(
        path: impl AsRef<Path>,
        expected_ref: &str,
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
        if query_meta(&conn, "graph_ref")?.as_deref() != Some(expected_ref) {
            return Ok(None);
        }

        Ok(Some(Self {
            conn,
            path: path.to_path_buf(),
        }))
    }

    pub fn open_current(
        path: impl AsRef<Path>,
        graph_ref: &str,
    ) -> Result<Option<Self>, KnowledgeError> {
        Self::open(path, graph_ref)
    }

    pub fn count_nodes(&self) -> Result<u64, KnowledgeError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM node", [], |row| row.get(0))
            .map_err(|error| sqlite_error(&self.path, "query graph sqlite node count", error))?;
        u64_from_i64(&self.path, "node count", count)
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

    pub fn search_exact_name(
        &self,
        name_lower: &str,
    ) -> Result<Vec<GraphIndexSearchRow>, KnowledgeError> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT id, node_type, kind, name, location, selector, scan_order
                FROM node
                WHERE name_lower = ?1
                ORDER BY scan_order ASC
                "#,
            )
            .map_err(|error| {
                sqlite_error(&self.path, "prepare graph sqlite exact-name search", error)
            })?;
        let rows = stmt
            .query_map(params![name_lower], graph_index_search_row_from_row)
            .map_err(|error| {
                sqlite_error(&self.path, "query graph sqlite exact-name search", error)
            })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| sqlite_error(&self.path, "read graph sqlite exact-name row", error))
    }

    pub fn search_location_prefix(
        &self,
        location_prefix_lower: &str,
    ) -> Result<Vec<GraphIndexSearchRow>, KnowledgeError> {
        let pattern = sqlite_like_prefix_pattern(location_prefix_lower);
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT id, node_type, kind, name, location, selector, scan_order
                FROM node
                WHERE location_lower LIKE ?1 ESCAPE '\'
                ORDER BY scan_order ASC
                "#,
            )
            .map_err(|error| {
                sqlite_error(
                    &self.path,
                    "prepare graph sqlite location-prefix search",
                    error,
                )
            })?;
        let rows = stmt
            .query_map(params![pattern], graph_index_search_row_from_row)
            .map_err(|error| {
                sqlite_error(
                    &self.path,
                    "query graph sqlite location-prefix search",
                    error,
                )
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            sqlite_error(&self.path, "read graph sqlite location-prefix row", error)
        })
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

    pub fn overview_counts(&self) -> Result<(usize, usize, usize), KnowledgeError> {
        Ok((
            self.node_count("dir")?,
            self.node_count("file")?,
            self.node_count("leaf")?,
        ))
    }

    pub fn overview_language_counts(&self) -> Result<HashMap<String, usize>, KnowledgeError> {
        self.string_count_map(
            r#"
            SELECT language, COUNT(*)
            FROM node
            WHERE node_type = 'file' AND language <> ''
            GROUP BY language
            ORDER BY language ASC
            "#,
            "query graph sqlite overview language counts",
        )
    }

    pub fn overview_symbol_kind_counts(&self) -> Result<HashMap<String, usize>, KnowledgeError> {
        self.string_count_map(
            r#"
            SELECT kind, COUNT(*)
            FROM node
            WHERE node_type = 'leaf' AND kind IS NOT NULL
            GROUP BY kind
            ORDER BY kind ASC
            "#,
            "query graph sqlite overview symbol kind counts",
        )
    }

    pub fn overview_dir_file_counts(&self) -> Result<BTreeMap<String, usize>, KnowledgeError> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT dir_key, COUNT(*)
                FROM (
                  SELECT CASE
                    WHEN trimmed_path = '' THEN '.'
                    WHEN instr(trimmed_path, '/') = 0 THEN '.'
                    ELSE substr(trimmed_path, 1, instr(trimmed_path, '/') - 1)
                  END AS dir_key
                  FROM (
                    SELECT ltrim(path, '/') AS trimmed_path
                    FROM file_summary
                  )
                )
                GROUP BY dir_key
                ORDER BY dir_key ASC
                "#,
            )
            .map_err(|error| {
                sqlite_error(
                    &self.path,
                    "prepare graph sqlite overview dir file counts",
                    error,
                )
            })?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| {
                sqlite_error(
                    &self.path,
                    "query graph sqlite overview dir file counts",
                    error,
                )
            })?;

        let mut counts = BTreeMap::new();
        for row in rows {
            let (key, count) = row.map_err(|error| {
                sqlite_error(
                    &self.path,
                    "read graph sqlite overview dir file count row",
                    error,
                )
            })?;
            counts.insert(
                key,
                usize_from_i64(&self.path, "overview dir file count", count)?,
            );
        }
        Ok(counts)
    }

    pub fn overview_top_files(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, String, usize)>, KnowledgeError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let limit = i64::try_from(limit).map_err(|_| {
            KnowledgeError::invalid_data("graph sqlite overview top-file limit exceeds i64")
        })?;
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT COALESCE(node.selector, 'file:' || file_summary.path) AS selector,
                       node.name,
                       file_summary.symbol_count
                FROM file_summary
                JOIN node ON node.id = file_summary.file_id
                ORDER BY file_summary.symbol_count DESC,
                         file_summary.path ASC,
                         selector ASC,
                         node.name ASC
                LIMIT ?1
                "#,
            )
            .map_err(|error| {
                sqlite_error(&self.path, "prepare graph sqlite overview top files", error)
            })?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|error| {
                sqlite_error(&self.path, "query graph sqlite overview top files", error)
            })?;

        let mut files = Vec::new();
        for row in rows {
            let (selector, name, symbol_count) = row.map_err(|error| {
                sqlite_error(&self.path, "read graph sqlite overview top-file row", error)
            })?;
            files.push((
                selector,
                name,
                usize_from_i64(&self.path, "overview top-file symbol count", symbol_count)?,
            ));
        }
        Ok(files)
    }

    fn node_count(&self, node_type: &str) -> Result<usize, KnowledgeError> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM node WHERE node_type = ?1",
                params![node_type],
                |row| row.get(0),
            )
            .map_err(|error| {
                sqlite_error(&self.path, "query graph sqlite overview count", error)
            })?;
        usize_from_i64(&self.path, "overview node count", count)
    }

    fn string_count_map(
        &self,
        sql: &str,
        action: &str,
    ) -> Result<HashMap<String, usize>, KnowledgeError> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|error| sqlite_error(&self.path, action, error))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| sqlite_error(&self.path, action, error))?;

        let mut counts = HashMap::new();
        for row in rows {
            let (key, count) = row
                .map_err(|error| sqlite_error(&self.path, "read graph sqlite count row", error))?;
            counts.insert(key, usize_from_i64(&self.path, "overview count", count)?);
        }
        Ok(counts)
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

fn usize_from_i64(path: &Path, label: &str, value: i64) -> Result<usize, KnowledgeError> {
    usize::try_from(value).map_err(|_| {
        KnowledgeError::invalid_data(format!(
            "graph sqlite index {} returned invalid {label} `{value}`",
            path.display()
        ))
    })
}

fn u64_from_i64(path: &Path, label: &str, value: i64) -> Result<u64, KnowledgeError> {
    u64::try_from(value).map_err(|_| {
        KnowledgeError::invalid_data(format!(
            "graph sqlite index {} returned invalid {label} `{value}`",
            path.display()
        ))
    })
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

fn graph_index_search_row_from_row(row: &Row<'_>) -> rusqlite::Result<GraphIndexSearchRow> {
    Ok(GraphIndexSearchRow {
        id: row.get(0)?,
        node_type: row.get(1)?,
        kind: row.get(2)?,
        name: row.get(3)?,
        location: row.get(4)?,
        selector: row.get(5)?,
        scan_order: row.get(6)?,
    })
}

fn sqlite_like_prefix_pattern(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len() + 1);
    for ch in input.chars() {
        match ch {
            '%' | '_' | '\\' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped.push('%');
    escaped
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

#[cfg(test)]
mod tests {
    use super::super::nodes::{
        BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode,
    };
    use super::super::object_store::GraphObjectStore;
    use super::*;

    #[test]
    fn open_missing_index_returns_none() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("missing.sqlite");

        let reader = GraphIndexReader::open(&path, "graph-a").expect("open missing");

        assert!(reader.is_none());
    }

    #[test]
    fn open_stale_ref_returns_none() {
        let (_temp_dir, store, current_ref) = write_fixture_index();

        let reader = GraphIndexReader::open(store.graph_sqlite_index_path(), "different-ref")
            .expect("open stale ref");

        assert!(reader.is_none());
        assert_ne!(current_ref.root_graph_hash, "different-ref");
    }

    #[test]
    fn open_stale_schema_returns_none() {
        let (_temp_dir, store, current_ref) = write_fixture_index();
        let index_path = store.graph_sqlite_index_path();
        let conn = Connection::open(&index_path).expect("open sqlite index for schema update");
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            [GRAPH_SQLITE_INDEX_SCHEMA_VERSION
                .saturating_sub(1)
                .to_string()],
        )
        .expect("update schema version");
        drop(conn);

        let reader =
            GraphIndexReader::open(&index_path, &current_ref.root_graph_hash).expect("open index");

        assert!(reader.is_none());
    }

    #[test]
    fn open_valid_index_counts_nodes_like_node_table() {
        let (_temp_dir, store, current_ref) = write_fixture_index();
        let index_path = store.graph_sqlite_index_path();
        let conn = Connection::open(&index_path).expect("open sqlite index");
        let expected_count: u64 = conn
            .query_row("SELECT COUNT(*) FROM node", [], |row| row.get::<_, i64>(0))
            .expect("count nodes")
            .try_into()
            .expect("node count is non-negative");
        drop(conn);

        let reader = GraphIndexReader::open(&index_path, &current_ref.root_graph_hash)
            .expect("open index")
            .expect("valid current index");

        assert_eq!(
            reader.count_nodes().expect("count reader nodes"),
            expected_count
        );
    }

    #[test]
    fn open_valid_index_uses_read_only_connection() {
        let (_temp_dir, store, current_ref) = write_fixture_index();
        let reader = GraphIndexReader::open(
            store.graph_sqlite_index_path(),
            &current_ref.root_graph_hash,
        )
        .expect("open index")
        .expect("valid current index");

        let write_result = reader.conn.execute(
            "INSERT INTO meta (key, value) VALUES ('read_only_test', 'should_fail')",
            [],
        );

        assert!(write_result.is_err());
    }

    fn write_fixture_index() -> (
        tempfile::TempDir,
        GraphObjectStore,
        super::super::object_store::CurrentRef,
    ) {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let store = GraphObjectStore::new(temp_dir.path());
        let current_ref = store.write_graph(&fixture_graph()).expect("write graph");
        (temp_dir, store, current_ref)
    }

    fn fixture_graph() -> CodebaseGraphV1 {
        CodebaseGraphV1 {
            root_dir_id: "dir-root".to_string(),
            dirs: vec![DirNode {
                base: base_node("dir-root", ".", "./", None),
                dir_children: Vec::new(),
                file_children: vec!["file-lib".to_string()],
            }],
            files: vec![FileNode {
                base: base_node("file-lib", "Lib.rs", "src/Lib.rs", Some("dir-root")),
                extension: Some("rs".to_string()),
                source_blob_hash: None,
                source: "pub fn greet() { helper(); }\n".to_string(),
                imports: Vec::new(),
                exports: vec!["greet".to_string()],
                re_exports: Vec::new(),
                leaf_children: vec!["leaf-greet".to_string()],
            }],
            leaves: vec![LeafNode {
                base: base_node("leaf-greet", "Greet", "src/Lib.rs#Greet", Some("file-lib")),
                kind: LeafKind::Function,
                source: "pub fn greet() { helper(); }\n".to_string(),
                source_blob_hash: None,
                source_hash: Some("source-hash".to_string()),
                file_hash_at_capture: Some("file-hash".to_string()),
                history: Vec::new(),
                input_signature: Vec::new(),
                output_signature: Vec::new(),
                start_line: Some(1),
                end_line: Some(1),
                children: Vec::new(),
            }],
        }
    }

    fn base_node(id: &str, name: &str, location: &str, parent_id: Option<&str>) -> BaseNodeFields {
        BaseNodeFields {
            id: id.to_string(),
            identity_key: id.to_string(),
            object_hash: None,
            name: name.to_string(),
            location: location.to_string(),
            language: "rust".to_string(),
            description: String::new(),
            parent_id: parent_id.map(str::to_string),
            is_locked: false,
            lineage_locked: false,
            lock_owner: None,
            lock_reason: String::new(),
        }
    }
}
