//! Command handler call-tree query.

use std::collections::VecDeque;

use rusqlite::{Connection, OptionalExtension, params};

use crate::query::callees;
use crate::{
    DEFAULT_TRACE_DEPTH, Graph, GraphError, RefConfidence, SymbolSpan, TRACE_NODE_CAP, TraceNode,
    TraceResult,
};

pub(crate) fn run(
    graph: &Graph,
    command: &str,
    depth: u8,
    min_confidence: RefConfidence,
) -> Result<TraceResult, GraphError> {
    let command = normalize_command_selector(command);
    graph.with_read_connection(|conn| {
        let Some(origin) = resolve_command_handler(conn, command)? else {
            return Ok(TraceResult::empty());
        };

        let max_depth = usize::from(if depth == 0 {
            DEFAULT_TRACE_DEPTH
        } else {
            depth
        });
        let mut arena = vec![TraceNodeBuilder::root(&origin)];
        let mut queue = VecDeque::from([(0usize, origin, 0usize)]);
        let mut truncated = false;

        'bfs: while let Some((node_index, symbol, distance)) = queue.pop_front() {
            if distance >= max_depth {
                continue;
            }

            let next_distance = distance + 1;
            for edge in callees::edges_for_symbol(conn, &symbol.span())? {
                if !confidence_visible_at_floor(edge.confidence.as_str(), min_confidence)? {
                    continue;
                }
                if arena.len() >= TRACE_NODE_CAP {
                    truncated = true;
                    break 'bfs;
                }

                let child_symbol = match edge.target_qualified.as_deref() {
                    Some(qualified) => resolve_symbol_by_qualified(conn, qualified)?,
                    None => None,
                };
                let child = TraceNodeBuilder::callee(
                    edge.target_name,
                    edge.target_qualified,
                    edge.confidence,
                );
                let child_index = arena.len();
                arena.push(child);
                arena[node_index].children.push(child_index);

                if next_distance < max_depth
                    && let Some(symbol) = child_symbol
                {
                    queue.push_back((child_index, symbol, next_distance));
                }
            }
        }

        Ok(TraceResult {
            root: Some(build_tree(&arena, 0)),
            truncated,
            visited_nodes: arena.len(),
        })
    })
}

fn normalize_command_selector(command: &str) -> &str {
    command
        .trim()
        .strip_prefix("command:")
        .map(str::trim)
        .unwrap_or_else(|| command.trim())
}

fn confidence_visible_at_floor(
    confidence: &str,
    min_confidence: RefConfidence,
) -> Result<bool, GraphError> {
    Ok(RefConfidence::from_db(confidence)?.visible_at_floor(min_confidence))
}

fn resolve_command_handler(
    conn: &Connection,
    command: &str,
) -> Result<Option<TraceSymbol>, GraphError> {
    conn.query_row(
        "SELECT s.file_path, s.name, s.qualified, s.span_start, s.span_end
         FROM commands c
         JOIN symbols s ON s.id = c.handler_symbol
         WHERE c.name = ?1
         LIMIT 1",
        params![command],
        trace_symbol_from_row,
    )
    .optional()
    .map_err(|source| GraphError::sqlite("resolve trace command handler", source))
}

fn resolve_symbol_by_qualified(
    conn: &Connection,
    qualified: &str,
) -> Result<Option<TraceSymbol>, GraphError> {
    conn.query_row(
        "SELECT file_path, name, qualified, span_start, span_end
         FROM symbols
         WHERE qualified = ?1
         ORDER BY id
         LIMIT 1",
        params![qualified],
        trace_symbol_from_row,
    )
    .optional()
    .map_err(|source| GraphError::sqlite("resolve trace callee symbol", source))
}

fn trace_symbol_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TraceSymbol> {
    Ok(TraceSymbol {
        file_path: row.get(0)?,
        name: row.get(1)?,
        qualified: row.get(2)?,
        span_start: row.get(3)?,
        span_end: row.get(4)?,
    })
}

fn build_tree(arena: &[TraceNodeBuilder], index: usize) -> TraceNode {
    let node = &arena[index];
    TraceNode {
        name: node.name.clone(),
        qualified_name: node.qualified_name.clone(),
        confidence: node.confidence.clone(),
        children: node
            .children
            .iter()
            .map(|child_index| build_tree(arena, *child_index))
            .collect(),
    }
}

#[derive(Debug, Clone)]
struct TraceSymbol {
    file_path: String,
    name: String,
    qualified: String,
    span_start: i64,
    span_end: i64,
}

impl TraceSymbol {
    fn span(&self) -> SymbolSpan {
        SymbolSpan {
            file_path: self.file_path.clone(),
            span_start: self.span_start,
            span_end: self.span_end,
        }
    }
}

#[derive(Debug, Clone)]
struct TraceNodeBuilder {
    name: String,
    qualified_name: Option<String>,
    confidence: Option<String>,
    children: Vec<usize>,
}

impl TraceNodeBuilder {
    fn root(symbol: &TraceSymbol) -> Self {
        Self {
            name: symbol.name.clone(),
            qualified_name: Some(symbol.qualified.clone()),
            confidence: None,
            children: Vec::new(),
        }
    }

    fn callee(name: String, qualified_name: Option<String>, confidence: String) -> Self {
        Self {
            name,
            qualified_name,
            confidence: Some(confidence),
            children: Vec::new(),
        }
    }
}

#[cfg(test)]
#[path = "tests/trace.rs"]
mod tests;
