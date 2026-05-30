//! Full-text search over indexed symbols, strings, and configs.

use std::fs;

use rusqlite::{Connection, params};
use serde::Serialize;

use crate::{Graph, GraphError};

/// Default number of matches returned by [`Graph::search`].
pub const DEFAULT_SEARCH_LIMIT: usize = 20;

/// Search request for the graph query surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// User query text passed through SQLite FTS5.
    pub query: String,
    /// Optional result-kind filter.
    pub kind: Option<SearchKind>,
    /// Optional language filter matched against `files.lang`.
    pub lang: Option<String>,
    /// Optional result limit. Defaults to [`DEFAULT_SEARCH_LIMIT`].
    pub limit: Option<usize>,
}

impl SearchQuery {
    /// Build a search query with default filters.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            kind: None,
            lang: None,
            limit: None,
        }
    }

    fn limit(&self) -> usize {
        self.limit.unwrap_or(DEFAULT_SEARCH_LIMIT)
    }
}

/// Search result kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchKind {
    /// Symbol definitions.
    Symbol,
    /// Notable string literals.
    String,
    /// Structured config keys.
    Config,
}

impl SearchKind {
    /// Parse a CLI/tool kind filter.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "symbol" => Some(Self::Symbol),
            "string" => Some(Self::String),
            "config" => Some(Self::Config),
            _ => None,
        }
    }
}

/// Search output matching `GRAPH_SPEC.md` section 9.1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchResult {
    /// Ranked matches.
    pub matches: Vec<Match>,
}

/// Search match returned by [`Graph::search`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Match {
    /// Symbol match.
    Symbol {
        /// Matched symbol name.
        name: String,
        /// Workspace-relative source path.
        path: String,
        /// One-based source line.
        line: usize,
    },
    /// String literal match.
    #[serde(rename = "string")]
    StringLiteral {
        /// Matched string value.
        value: String,
        /// Workspace-relative source path.
        path: String,
        /// One-based source line.
        line: usize,
    },
    /// Config key match.
    Config {
        /// Matched config key.
        value: String,
        /// Workspace-relative source path.
        path: String,
        /// One-based source line.
        line: usize,
    },
}

pub(crate) fn run(graph: &Graph, q: &SearchQuery) -> Result<SearchResult, GraphError> {
    let query = q.query.trim();
    let limit = q.limit();
    if query.is_empty() || limit == 0 {
        return Ok(SearchResult {
            matches: Vec::new(),
        });
    }

    let conn = Connection::open(graph.db_path.path())
        .map_err(|source| GraphError::sqlite("open graph database for search", source))?;
    let raw_matches = query_matches(&conn, q, query, limit)?;
    let matches = raw_matches
        .into_iter()
        .map(|raw| materialize_match(graph, raw))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SearchResult { matches })
}

fn query_matches(
    conn: &Connection,
    q: &SearchQuery,
    query: &str,
    limit: usize,
) -> Result<Vec<RawMatch>, GraphError> {
    let mut arms = Vec::new();
    for kind in [SearchKind::Symbol, SearchKind::String, SearchKind::Config] {
        if q.kind.is_some_and(|filter| filter != kind) {
            continue;
        }
        arms.push(sql_arm(kind, q.lang.is_some()));
    }

    if arms.is_empty() {
        return Ok(Vec::new());
    }

    let limit_index = if q.lang.is_some() { 3 } else { 2 };
    let sql = format!(
        "SELECT kind, label, path, span_start, line
         FROM ({})
         ORDER BY rank
         LIMIT ?{limit_index}",
        arms.join(" UNION ALL ")
    );
    let fts = fts_query(query);
    let limit = usize_to_i64("convert search result limit", limit)?;
    let mut stmt = conn
        .prepare_cached(&sql)
        .map_err(|source| GraphError::sqlite("prepare graph search query", source))?;

    let rows = if let Some(lang) = q.lang.as_ref() {
        stmt.query_map(params![fts, lang, limit], raw_match_from_row)
            .map_err(|source| GraphError::sqlite("query graph search matches", source))?
            .collect::<Result<Vec<_>, _>>()
    } else {
        stmt.query_map(params![fts, limit], raw_match_from_row)
            .map_err(|source| GraphError::sqlite("query graph search matches", source))?
            .collect::<Result<Vec<_>, _>>()
    };

    rows.map_err(|source| GraphError::sqlite("collect graph search matches", source))
}

fn sql_arm(kind: SearchKind, filter_lang: bool) -> &'static str {
    match (kind, filter_lang) {
        (SearchKind::Symbol, true) => {
            "SELECT 'symbol' AS kind, s.name AS label, s.file_path AS path,
                    s.span_start AS span_start, NULL AS line, bm25(symbols_fts) AS rank
             FROM symbols_fts
             JOIN symbols s ON s.id = symbols_fts.rowid
             JOIN files f ON f.path = s.file_path
             WHERE symbols_fts MATCH ?1 AND f.lang = ?2"
        }
        (SearchKind::Symbol, false) => {
            "SELECT 'symbol' AS kind, s.name AS label, s.file_path AS path,
                    s.span_start AS span_start, NULL AS line, bm25(symbols_fts) AS rank
             FROM symbols_fts
             JOIN symbols s ON s.id = symbols_fts.rowid
             JOIN files f ON f.path = s.file_path
             WHERE symbols_fts MATCH ?1"
        }
        (SearchKind::String, true) => {
            "SELECT 'string' AS kind, st.value AS label, st.file_path AS path,
                    NULL AS span_start, st.line AS line, bm25(strings_fts) AS rank
             FROM strings_fts
             JOIN strings st ON st.id = strings_fts.rowid
             JOIN files f ON f.path = st.file_path
             WHERE strings_fts MATCH ?1 AND f.lang = ?2"
        }
        (SearchKind::String, false) => {
            "SELECT 'string' AS kind, st.value AS label, st.file_path AS path,
                    NULL AS span_start, st.line AS line, bm25(strings_fts) AS rank
             FROM strings_fts
             JOIN strings st ON st.id = strings_fts.rowid
             JOIN files f ON f.path = st.file_path
             WHERE strings_fts MATCH ?1"
        }
        (SearchKind::Config, true) => {
            "SELECT 'config' AS kind, c.key AS label, c.file_path AS path,
                    NULL AS span_start, c.line AS line, bm25(configs_fts) AS rank
             FROM configs_fts
             JOIN configs c ON c.id = configs_fts.rowid
             JOIN files f ON f.path = c.file_path
             WHERE configs_fts MATCH ?1 AND f.lang = ?2"
        }
        (SearchKind::Config, false) => {
            "SELECT 'config' AS kind, c.key AS label, c.file_path AS path,
                    NULL AS span_start, c.line AS line, bm25(configs_fts) AS rank
             FROM configs_fts
             JOIN configs c ON c.id = configs_fts.rowid
             JOIN files f ON f.path = c.file_path
             WHERE configs_fts MATCH ?1"
        }
    }
}

fn fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawMatch {
    kind: SearchKind,
    label: String,
    path: String,
    span_start: Option<i64>,
    line: Option<i64>,
}

fn raw_match_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawMatch> {
    let kind = match row.get::<_, String>(0)?.as_str() {
        "symbol" => SearchKind::Symbol,
        "string" => SearchKind::String,
        "config" => SearchKind::Config,
        _ => unreachable!("search query only emits known kinds"),
    };
    Ok(RawMatch {
        kind,
        label: row.get(1)?,
        path: row.get(2)?,
        span_start: row.get(3)?,
        line: row.get(4)?,
    })
}

fn materialize_match(graph: &Graph, raw: RawMatch) -> Result<Match, GraphError> {
    let line = match (raw.line, raw.span_start) {
        (Some(line), _) => i64_to_usize("convert graph search line", line)?,
        (None, Some(span_start)) => symbol_line(graph, raw.path.as_str(), span_start)?,
        (None, None) => {
            return Err(GraphError::invalid_data(
                "materialize graph search match",
                "match row has neither line nor span_start",
            ));
        }
    };

    Ok(match raw.kind {
        SearchKind::Symbol => Match::Symbol {
            name: raw.label,
            path: raw.path,
            line,
        },
        SearchKind::String => Match::StringLiteral {
            value: raw.label,
            path: raw.path,
            line,
        },
        SearchKind::Config => Match::Config {
            value: raw.label,
            path: raw.path,
            line,
        },
    })
}

fn symbol_line(graph: &Graph, path: &str, span_start: i64) -> Result<usize, GraphError> {
    let span_start = i64_to_usize("convert graph search symbol span", span_start)?;
    let source_path = graph.worktree_root.join(path);
    let bytes = fs::read(source_path.as_path()).map_err(|source| {
        GraphError::io("read source file for graph search", source_path, source)
    })?;
    if span_start > bytes.len() {
        return Err(GraphError::invalid_data(
            "read source line for graph search",
            format!(
                "span_start {span_start} is beyond source length {} for {path}",
                bytes.len()
            ),
        ));
    }
    Ok(bytes[..span_start]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1)
}

fn usize_to_i64(operation: &'static str, value: usize) -> Result<i64, GraphError> {
    i64::try_from(value).map_err(|source| GraphError::invalid_data(operation, source.to_string()))
}

fn i64_to_usize(operation: &'static str, value: i64) -> Result<usize, GraphError> {
    usize::try_from(value).map_err(|source| GraphError::invalid_data(operation, source.to_string()))
}

#[cfg(test)]
#[path = "tests/search.rs"]
mod tests;
