//! Outbound call-edge query.

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
    edges_for_symbol(&conn, &symbol)
}

pub(crate) fn edges_for_symbol(
    conn: &Connection,
    symbol: &SymbolSpan,
) -> Result<Vec<CalleeEdge>, GraphError> {
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
            Ok(CalleeEdge {
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
