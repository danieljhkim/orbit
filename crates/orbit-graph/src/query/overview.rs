//! Repository shape summary query.
//!
//! Aggregates the `files` and `symbols` tables into language and symbol-kind
//! counts, optionally scoped to a `dir:` or `file:` selector. `summary` returns
//! counts plus the highest-symbol files; `full` additionally lists every
//! in-scope file with its symbols. See ORB-00389.

use std::collections::BTreeMap;

use orbit_graph_extract::Selector;
use rusqlite::{Connection, Row, params};

use crate::{Graph, GraphError, OverviewFile, OverviewFormat, OverviewResult, OverviewSymbol};

/// Files surfaced in `summary` format, ranked by symbol count.
const SUMMARY_TOP_FILES: usize = 20;

pub(crate) fn run(
    graph: &Graph,
    scope: Option<&Selector>,
    format: OverviewFormat,
) -> Result<OverviewResult, GraphError> {
    let scope = ScopeFilter::from_selector(scope)?;
    graph.with_read_connection(|conn| {
        let languages = language_counts(conn, &scope)?;
        let symbol_kinds = symbol_kind_counts(conn, &scope)?;
        let files = match format {
            OverviewFormat::Summary => top_files(conn, &scope)?,
            OverviewFormat::Full => full_files(conn, &scope)?,
        };
        Ok(OverviewResult {
            format,
            scope: scope.echo,
            total_files: languages.values().sum(),
            total_symbols: symbol_kinds.values().sum(),
            languages,
            symbol_kinds,
            files,
        })
    })
}

struct ScopeFilter {
    kind: ScopeKind,
    echo: Option<String>,
}

enum ScopeKind {
    All,
    FileEq(String),
    DirLike(String),
}

impl ScopeFilter {
    fn from_selector(scope: Option<&Selector>) -> Result<Self, GraphError> {
        match scope {
            None => Ok(Self {
                kind: ScopeKind::All,
                echo: None,
            }),
            Some(Selector::File { path }) => Ok(Self {
                kind: ScopeKind::FileEq(path.clone()),
                echo: Some(path.clone()),
            }),
            Some(Selector::Dir { path }) => Ok(Self {
                kind: ScopeKind::DirLike(format!("{}/%", path.trim_end_matches('/'))),
                echo: Some(path.clone()),
            }),
            Some(_) => Err(GraphError::invalid_data(
                "resolve overview scope",
                "overview scope must be a `dir:` or `file:` selector",
            )),
        }
    }

    /// Build a `WHERE` fragment constraining `column`, and the bound parameter.
    fn predicate(&self, column: &str) -> (String, Option<&str>) {
        match &self.kind {
            ScopeKind::All => (String::new(), None),
            ScopeKind::FileEq(path) => (format!(" WHERE {column} = ?1"), Some(path.as_str())),
            ScopeKind::DirLike(pattern) => {
                (format!(" WHERE {column} LIKE ?1"), Some(pattern.as_str()))
            }
        }
    }
}

fn language_counts(
    conn: &Connection,
    scope: &ScopeFilter,
) -> Result<BTreeMap<String, usize>, GraphError> {
    let (where_clause, param) = scope.predicate("path");
    let sql = format!("SELECT lang, count(*) FROM files{where_clause} GROUP BY lang");
    grouped_counts(conn, &sql, param, "language counts")
}

fn symbol_kind_counts(
    conn: &Connection,
    scope: &ScopeFilter,
) -> Result<BTreeMap<String, usize>, GraphError> {
    let (where_clause, param) = scope.predicate("file_path");
    let sql = format!("SELECT kind, count(*) FROM symbols{where_clause} GROUP BY kind");
    grouped_counts(conn, &sql, param, "symbol kind counts")
}

fn grouped_counts(
    conn: &Connection,
    sql: &str,
    param: Option<&str>,
    op: &'static str,
) -> Result<BTreeMap<String, usize>, GraphError> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|source| GraphError::sqlite(op, source))?;
    let collected: Vec<(String, i64)> = match param {
        Some(value) => stmt.query_map(params![value], grouped_row),
        None => stmt.query_map([], grouped_row),
    }
    .map_err(|source| GraphError::sqlite(op, source))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|source| GraphError::sqlite(op, source))?;

    let mut counts = BTreeMap::new();
    for (key, count) in collected {
        counts.insert(key, count_to_usize(count, op)?);
    }
    Ok(counts)
}

fn grouped_row(row: &Row<'_>) -> rusqlite::Result<(String, i64)> {
    Ok((row.get(0)?, row.get(1)?))
}

fn top_files(conn: &Connection, scope: &ScopeFilter) -> Result<Vec<OverviewFile>, GraphError> {
    let (where_clause, param) = scope.predicate("f.path");
    let sql = format!(
        "SELECT f.path, f.lang, count(s.id)
         FROM files f
         LEFT JOIN symbols s ON s.file_path = f.path{where_clause}
         GROUP BY f.path, f.lang
         ORDER BY count(s.id) DESC, f.path
         LIMIT {SUMMARY_TOP_FILES}"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|source| GraphError::sqlite("prepare overview top files", source))?;
    let rows = match param {
        Some(value) => stmt.query_map(params![value], top_file_row),
        None => stmt.query_map([], top_file_row),
    }
    .map_err(|source| GraphError::sqlite("query overview top files", source))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|source| GraphError::sqlite("collect overview top files", source))?;

    rows.into_iter()
        .map(|(path, lang, count)| {
            Ok(OverviewFile {
                path,
                lang,
                symbol_count: count_to_usize(count, "overview top files")?,
                symbols: Vec::new(),
            })
        })
        .collect()
}

fn top_file_row(row: &Row<'_>) -> rusqlite::Result<(String, String, i64)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
}

fn full_files(conn: &Connection, scope: &ScopeFilter) -> Result<Vec<OverviewFile>, GraphError> {
    let (where_clause, param) = scope.predicate("f.path");
    let sql = format!(
        "SELECT f.path, f.lang, s.name, s.kind, s.qualified
         FROM files f
         LEFT JOIN symbols s ON s.file_path = f.path{where_clause}
         ORDER BY f.path, s.span_start, s.id"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|source| GraphError::sqlite("prepare overview full files", source))?;
    let rows = match param {
        Some(value) => stmt.query_map(params![value], full_file_row),
        None => stmt.query_map([], full_file_row),
    }
    .map_err(|source| GraphError::sqlite("query overview full files", source))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|source| GraphError::sqlite("collect overview full files", source))?;

    let mut files: Vec<OverviewFile> = Vec::new();
    for row in rows {
        if files.last().map(|f| f.path.as_str()) != Some(row.path.as_str()) {
            files.push(OverviewFile {
                path: row.path,
                lang: row.lang,
                symbol_count: 0,
                symbols: Vec::new(),
            });
        }
        if let (Some(name), Some(kind), Some(qualified)) = (row.name, row.kind, row.qualified) {
            let Some(file) = files.last_mut() else {
                continue;
            };
            file.symbols.push(OverviewSymbol {
                name,
                kind,
                qualified,
            });
            file.symbol_count += 1;
        }
    }
    Ok(files)
}

fn full_file_row(row: &Row<'_>) -> rusqlite::Result<FullFileRow> {
    Ok(FullFileRow {
        path: row.get(0)?,
        lang: row.get(1)?,
        name: row.get(2)?,
        kind: row.get(3)?,
        qualified: row.get(4)?,
    })
}

struct FullFileRow {
    path: String,
    lang: String,
    name: Option<String>,
    kind: Option<String>,
    qualified: Option<String>,
}

fn count_to_usize(count: i64, op: &'static str) -> Result<usize, GraphError> {
    usize::try_from(count).map_err(|source| GraphError::invalid_data(op, source.to_string()))
}

#[cfg(test)]
#[path = "tests/overview.rs"]
mod tests;
