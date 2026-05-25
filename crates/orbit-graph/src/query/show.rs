//! Selector resolution and bounded source reads.

use std::fs;

use orbit_graph_extract::Selector;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::{Graph, GraphError};

/// Default maximum source bytes returned by [`Graph::show`].
///
/// The value keeps a single result comfortably inside agent context while
/// still covering typical functions, files, and command handlers.
pub const DEFAULT_SHOW_MAX_BYTES: usize = 64 * 1024;

/// Source and metadata view returned by [`Graph::show`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeView {
    /// UTF-8 source text from the resolved span, bounded by the caller's byte budget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Source bytes from the resolved span when the payload is not valid UTF-8.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
    /// Metadata for the resolved source span.
    pub metadata: NodeMetadata,
}

/// Metadata for a [`NodeView`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeMetadata {
    /// Workspace-relative source path.
    pub file: String,
    /// Full resolved source span before byte-budget truncation.
    pub span: SourceSpan,
    /// Resolved node kind.
    pub kind: String,
    /// Display name when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Qualified symbol name when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualified: Option<String>,
    /// Whether the returned `text` or fallback `bytes` is shorter than the resolved source span.
    pub truncated: bool,
}

/// Byte span in a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SourceSpan {
    /// Inclusive byte start.
    pub start: usize,
    /// Exclusive byte end.
    pub end: usize,
}

pub(crate) fn run(
    graph: &Graph,
    selector: &Selector,
    max_bytes: usize,
) -> Result<Option<NodeView>, GraphError> {
    let conn = Connection::open(graph.db_path.path())
        .map_err(|source| GraphError::sqlite("open graph database for show", source))?;
    let Some(resolved) = resolve_selector(&conn, selector)? else {
        return Ok(None);
    };
    materialize_view(graph, resolved, max_bytes).map(Some)
}

fn resolve_selector(
    conn: &Connection,
    selector: &Selector,
) -> Result<Option<ResolvedView>, GraphError> {
    match selector {
        Selector::Symbol { path, symbol, kind } => resolve_symbol(conn, path, symbol, kind),
        Selector::File { path } => resolve_file(conn, path),
        Selector::Module { qualified } => resolve_module(conn, qualified),
        Selector::Command { name } => resolve_command(conn, name),
        Selector::Dir { .. } => Ok(None),
    }
}

fn resolve_symbol(
    conn: &Connection,
    path: &str,
    symbol: &str,
    kind: &str,
) -> Result<Option<ResolvedView>, GraphError> {
    conn.query_row(
        "SELECT file_path, span_start, span_end, kind, name, qualified
         FROM symbols
         WHERE file_path = ?1
           AND kind = ?3
           AND (name = ?2 OR qualified = ?2)
         ORDER BY CASE WHEN qualified = ?2 THEN 0 WHEN name = ?2 THEN 1 ELSE 2 END, id
         LIMIT 1",
        params![path, symbol, kind],
        resolved_symbol_from_row,
    )
    .optional()
    .map_err(|source| GraphError::sqlite("resolve graph symbol selector", source))
}

fn resolve_file(conn: &Connection, path: &str) -> Result<Option<ResolvedView>, GraphError> {
    conn.query_row(
        "SELECT path, 0, byte_len, 'file', path, NULL
         FROM files
         WHERE path = ?1",
        params![path],
        resolved_symbol_from_row,
    )
    .optional()
    .map_err(|source| GraphError::sqlite("resolve graph file selector", source))
}

fn resolve_module(conn: &Connection, qualified: &str) -> Result<Option<ResolvedView>, GraphError> {
    conn.query_row(
        "SELECT file_path, span_start, span_end, kind, name, qualified
         FROM symbols
         WHERE kind = 'module'
           AND (qualified = ?1 OR name = ?1)
         ORDER BY CASE WHEN qualified = ?1 THEN 0 WHEN name = ?1 THEN 1 ELSE 2 END, id
         LIMIT 1",
        params![qualified],
        resolved_symbol_from_row,
    )
    .optional()
    .map_err(|source| GraphError::sqlite("resolve graph module selector", source))
}

fn resolve_command(conn: &Connection, name: &str) -> Result<Option<ResolvedView>, GraphError> {
    conn.query_row(
        "SELECT c.file_path,
                COALESCE(s.span_start, c.span_start) AS span_start,
                COALESCE(s.span_end, f.byte_len) AS span_end,
                'command' AS kind,
                c.name AS name,
                s.qualified AS qualified
         FROM commands c
         JOIN files f ON f.path = c.file_path
         LEFT JOIN symbols s ON s.id = c.handler_symbol
         WHERE c.name = ?1
         ORDER BY c.name
         LIMIT 1",
        params![name],
        resolved_symbol_from_row,
    )
    .optional()
    .map_err(|source| GraphError::sqlite("resolve graph command selector", source))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedView {
    file: String,
    span_start: i64,
    span_end: i64,
    kind: String,
    name: Option<String>,
    qualified: Option<String>,
}

fn resolved_symbol_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ResolvedView> {
    Ok(ResolvedView {
        file: row.get(0)?,
        span_start: row.get(1)?,
        span_end: row.get(2)?,
        kind: row.get(3)?,
        name: row.get(4)?,
        qualified: row.get(5)?,
    })
}

fn materialize_view(
    graph: &Graph,
    resolved: ResolvedView,
    max_bytes: usize,
) -> Result<NodeView, GraphError> {
    let source_path = graph.worktree_root.join(resolved.file.as_str());
    let source = fs::read(source_path.as_path())
        .map_err(|source| GraphError::io("read source file for graph show", source_path, source))?;
    let span = validate_span(
        "read source span for graph show",
        resolved.span_start,
        resolved.span_end,
        source.len(),
        resolved.file.as_str(),
    )?;
    let full = &source[span.start..span.end];
    let payload = source_payload(full, max_bytes);
    let truncated = payload.byte_count < full.len();

    Ok(NodeView {
        text: payload.text,
        bytes: payload.bytes,
        metadata: NodeMetadata {
            file: resolved.file,
            span,
            kind: resolved.kind,
            name: resolved.name,
            qualified: resolved.qualified,
            truncated,
        },
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourcePayload {
    text: Option<String>,
    bytes: Option<Vec<u8>>,
    byte_count: usize,
}

fn source_payload(full: &[u8], max_bytes: usize) -> SourcePayload {
    let capped_len = full.len().min(max_bytes);
    match std::str::from_utf8(full) {
        Ok(source) => {
            let mut text_len = capped_len;
            while !source.is_char_boundary(text_len) {
                text_len -= 1;
            }
            SourcePayload {
                text: Some(source[..text_len].to_string()),
                bytes: None,
                byte_count: text_len,
            }
        }
        Err(_) => SourcePayload {
            text: None,
            bytes: Some(full[..capped_len].to_vec()),
            byte_count: capped_len,
        },
    }
}

fn validate_span(
    operation: &'static str,
    start: i64,
    end: i64,
    source_len: usize,
    file: &str,
) -> Result<SourceSpan, GraphError> {
    let start = i64_to_usize(operation, start)?;
    let end = i64_to_usize(operation, end)?;
    if start > end || end > source_len {
        return Err(GraphError::invalid_data(
            operation,
            format!("invalid span {start}..{end} for {file} with {source_len} bytes"),
        ));
    }
    Ok(SourceSpan { start, end })
}

fn i64_to_usize(operation: &'static str, value: i64) -> Result<usize, GraphError> {
    usize::try_from(value).map_err(|source| GraphError::invalid_data(operation, source.to_string()))
}

#[cfg(test)]
#[path = "tests/show.rs"]
mod tests;
