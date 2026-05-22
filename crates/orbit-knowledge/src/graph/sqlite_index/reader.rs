//! `GraphIndexReader` and all read/query paths over the SQLite graph index.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use super::GRAPH_SQLITE_INDEX_SCHEMA_VERSION;
use super::rows::{
    GraphIndexNodeRow, GraphIndexSearchRow, graph_index_node_from_row,
    graph_index_search_row_from_row, sqlite_like_substring_pattern,
};
use super::writer::{
    graph_index_debug_force_fallback, query_meta, sqlite_error, u64_from_i64, usize_from_i64,
};
use crate::error::KnowledgeError;

pub struct GraphIndexReader {
    conn: Connection,
    path: PathBuf,
}

impl GraphIndexReader {
    pub fn open(
        path: impl AsRef<Path>,
        expected_ref: &str,
    ) -> Result<Option<Self>, KnowledgeError> {
        if graph_index_debug_force_fallback() {
            return Ok(None);
        }

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

    /// Returns rows whose `name_lower` OR `location_lower` contains
    /// `query_lower` as a substring, ordered by `scan_order` (dirs before
    /// files before leaves, matching the navigator's scan).
    ///
    /// This mirrors `GraphContextService::node_candidate_matches` for the
    /// default-ranking search path: empty query matches every row (browse
    /// mode); non-empty query matches via `LIKE '%q%'` on either column.
    /// SQLite cannot use the `name_lower` / `location_lower` btree indexes for
    /// leading-wildcard LIKE, but a single-file scan over the compact node
    /// table is still substantially cheaper than walking the by-id JSON
    /// objects in the fallback.
    ///
    /// Replaces the prior `search_exact_name` / `search_location_prefix`
    /// methods, whose exact-equality and prefix semantics violated
    /// output-equivalence with the fallback (T20260510-1).
    pub fn search_substring(
        &self,
        query_lower: &str,
        limit: usize,
    ) -> Result<Vec<GraphIndexSearchRow>, KnowledgeError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).map_err(|_| {
            KnowledgeError::invalid_data("graph sqlite search limit exceeds i64".to_string())
        })?;

        if query_lower.is_empty() {
            // Browse mode: return everything in scan order, capped at `limit`.
            let mut stmt = self
                .conn
                .prepare(
                    r#"
                    SELECT id, node_type, kind, name, location, selector, scan_order
                    FROM node
                    ORDER BY scan_order ASC
                    LIMIT ?1
                    "#,
                )
                .map_err(|error| {
                    sqlite_error(&self.path, "prepare graph sqlite browse search", error)
                })?;
            let rows = stmt
                .query_map(params![limit], graph_index_search_row_from_row)
                .map_err(|error| {
                    sqlite_error(&self.path, "query graph sqlite browse search", error)
                })?;
            return rows
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| sqlite_error(&self.path, "read graph sqlite browse row", error));
        }

        let pattern = sqlite_like_substring_pattern(query_lower);
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT id, node_type, kind, name, location, selector, scan_order
                FROM node
                WHERE name_lower LIKE ?1 ESCAPE '\'
                   OR location_lower LIKE ?1 ESCAPE '\'
                ORDER BY scan_order ASC
                LIMIT ?2
                "#,
            )
            .map_err(|error| {
                sqlite_error(&self.path, "prepare graph sqlite substring search", error)
            })?;
        let rows = stmt
            .query_map(params![pattern, limit], graph_index_search_row_from_row)
            .map_err(|error| {
                sqlite_error(&self.path, "query graph sqlite substring search", error)
            })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| sqlite_error(&self.path, "read graph sqlite substring row", error))
    }

    /// Returns the forward children of `parent_id` in stored order, matching
    /// `GraphNavigator::get_children` semantics.
    ///
    /// Uses the `child` edge table (forward pointers) rather than
    /// `node.parent_id` (reverse pointers). The graph data model stamps every
    /// leaf's `parent_id` with the containing file id even when the leaf's
    /// semantic parent is another leaf, so a reverse-pointer query would miss
    /// nested leaf-leaf relationships. See T20260510-2.
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
                SELECT n.id, n.node_type, n.kind, n.location, n.parent_id, n.selector, n.object_hash
                FROM child c
                JOIN node n ON n.id = c.child_id
                WHERE c.parent_id = ?1
                ORDER BY c.ordinal ASC, n.id ASC
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
