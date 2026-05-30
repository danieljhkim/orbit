//! Outbound call-edge query.

use std::fs;
use std::path::{Path, PathBuf};

use orbit_graph_extract::Selector;
use rusqlite::{Connection, params};

use crate::{CalleeEdge, Graph, GraphError, SymbolSpan, resolve_symbol_span};

pub(crate) fn run(graph: &Graph, sel: &Selector) -> Result<Vec<CalleeEdge>, GraphError> {
    let symbol = match resolve_symbol_span(graph.db_path.path(), sel)? {
        Some(s) => s,
        None => return Ok(vec![]),
    };

    let conn = Connection::open(graph.db_path.path())
        .map_err(|source| GraphError::sqlite("open graph database for callees", source))?;
    let edges = edges_for_symbol(&conn, &symbol)?;
    materialize_edges(
        graph.worktree_root.as_path(),
        symbol.file_path.as_str(),
        edges,
    )
}

pub(crate) fn edges_for_symbol(
    conn: &Connection,
    symbol: &SymbolSpan,
) -> Result<Vec<StoredCalleeEdge>, GraphError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT target_name, target_qualified, confidence, from_span_start
             FROM refs
             WHERE from_file = ?1
               AND from_span_start >= ?2
               AND from_span_end <= ?3
               AND kind = 'call'
             ORDER BY from_span_start, id",
        )
        .map_err(|source| GraphError::sqlite("prepare callees query", source))?;

    stmt.query_map(
        params![symbol.file_path, symbol.span_start, symbol.span_end],
        |row| {
            Ok(StoredCalleeEdge {
                target_name: row.get(0)?,
                target_qualified: row.get(1)?,
                confidence: row.get(2)?,
                from_span: row.get(3)?,
            })
        },
    )
    .map_err(|source| GraphError::sqlite("execute callees query", source))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|source| GraphError::sqlite("collect callees edges", source))
}

#[derive(Debug, Clone)]
pub(crate) struct StoredCalleeEdge {
    pub(crate) target_name: String,
    pub(crate) target_qualified: Option<String>,
    pub(crate) confidence: String,
    pub(crate) from_span: i64,
}

fn materialize_edges(
    worktree_root: &Path,
    file_path: &str,
    edges: Vec<StoredCalleeEdge>,
) -> Result<Vec<CalleeEdge>, GraphError> {
    if edges.is_empty() {
        return Ok(Vec::new());
    }

    let source_path = source_path(worktree_root, file_path);
    let bytes = fs::read(source_path.as_path()).map_err(|source| {
        GraphError::io(
            "read source file for graph callee line",
            source_path,
            source,
        )
    })?;
    let lines = LineIndex::new(bytes);
    edges
        .into_iter()
        .map(|edge| {
            let from_span = usize::try_from(edge.from_span).map_err(|source| {
                GraphError::invalid_data("compute graph callee line", source.to_string())
            })?;
            Ok(CalleeEdge {
                target_name: edge.target_name,
                target_qualified: edge.target_qualified,
                confidence: edge.confidence,
                line: lines.line_for(from_span),
            })
        })
        .collect()
}

fn source_path(worktree_root: &Path, file: &str) -> PathBuf {
    let path = Path::new(file);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        worktree_root.join(path)
    }
}

struct LineIndex {
    byte_len: usize,
    line_starts: Vec<usize>,
}

impl LineIndex {
    fn new(bytes: Vec<u8>) -> Self {
        let mut line_starts = vec![0];
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(index + 1);
            }
        }
        Self {
            byte_len: bytes.len(),
            line_starts,
        }
    }

    fn line_for(&self, byte_offset: usize) -> usize {
        let capped = byte_offset.min(self.byte_len);
        self.line_starts
            .partition_point(|line_start| *line_start <= capped)
    }
}
