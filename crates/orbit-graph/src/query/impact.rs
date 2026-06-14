//! Bounded blast-radius traversal.

use std::collections::{HashSet, VecDeque};

use orbit_graph_extract::Selector;
use rusqlite::{Connection, OptionalExtension, params};

use crate::query::callees;
use crate::{
    DEFAULT_IMPACT_DEPTH, Graph, GraphError, IMPACT_NODE_CAP, ImpactEntry, ImpactFallback,
    ImpactResult, RefConfidence, RefKind, SymbolSpan,
};

pub(crate) fn run(
    graph: &Graph,
    sel: &Selector,
    depth: u8,
    min_confidence: RefConfidence,
) -> Result<ImpactResult, GraphError> {
    graph.with_read_connection(|conn| {
        let Some(origin) = resolve_selector(conn, sel)? else {
            return Ok(empty_result());
        };

        let max_depth = usize::from(if depth == 0 {
            DEFAULT_IMPACT_DEPTH
        } else {
            depth
        });
        let traversal = traverse(conn, origin.clone(), max_depth, min_confidence)?;
        let fallback = maybe_fuzzy_fallback(
            conn,
            &origin,
            max_depth,
            min_confidence,
            traversal.touched.as_slice(),
        )?;

        Ok(ImpactResult {
            visited_nodes: traversal.touched.len(),
            touched: traversal.touched,
            truncated: traversal.truncated,
            fallback,
        })
    })
}

fn empty_result() -> ImpactResult {
    ImpactResult {
        touched: Vec::new(),
        truncated: false,
        visited_nodes: 0,
        fallback: None,
    }
}

fn traverse(
    conn: &Connection,
    origin: ImpactSymbol,
    max_depth: usize,
    min_confidence: RefConfidence,
) -> Result<ImpactTraversal, GraphError> {
    let mut queue = VecDeque::from([(origin.clone(), 0usize)]);
    let mut seen = HashSet::from([origin.qualified]);
    let mut touched = Vec::new();
    let mut truncated = false;

    'bfs: while let Some((symbol, distance)) = queue.pop_front() {
        if distance >= max_depth {
            continue;
        }
        let next_distance = distance + 1;
        for neighbor in neighbors(conn, &symbol, min_confidence)? {
            if seen.contains(neighbor.qualified_name.as_str()) {
                continue;
            }
            if touched.len() >= IMPACT_NODE_CAP {
                truncated = true;
                break 'bfs;
            }

            seen.insert(neighbor.qualified_name.clone());
            if next_distance < max_depth
                && let Some(next_symbol) =
                    resolve_symbol_by_qualified(conn, neighbor.qualified_name.as_str())?
            {
                queue.push_back((next_symbol, next_distance));
            }
            touched.push(ImpactEntry {
                qualified_name: neighbor.qualified_name,
                distance: next_distance,
                edge_kind: neighbor.edge_kind,
            });
        }
    }

    Ok(ImpactTraversal { touched, truncated })
}

fn maybe_fuzzy_fallback(
    conn: &Connection,
    origin: &ImpactSymbol,
    max_depth: usize,
    min_confidence: RefConfidence,
    touched: &[ImpactEntry],
) -> Result<Option<ImpactFallback>, GraphError> {
    if !touched.is_empty() || min_confidence == RefConfidence::FuzzyName {
        return Ok(None);
    }

    let fallback = traverse(conn, origin.clone(), max_depth, RefConfidence::FuzzyName)?;
    if fallback.touched.is_empty() {
        return Ok(None);
    }

    let note = format!(
        "No impacted symbols resolved at the `{}` confidence floor; showing {} match(es) found at \
         the `fuzzy_name` fallback floor. These are name-only matches and may include unrelated \
         symbols sharing the name; check each entry before acting.",
        confidence_label(min_confidence),
        fallback.touched.len(),
    );
    Ok(Some(ImpactFallback {
        confidence: RefConfidence::FuzzyName,
        visited_nodes: fallback.touched.len(),
        touched: fallback.touched,
        truncated: fallback.truncated,
        note,
    }))
}

fn confidence_label(confidence: RefConfidence) -> &'static str {
    match confidence {
        RefConfidence::Exact => "exact",
        RefConfidence::ImportResolved => "import_resolved",
        RefConfidence::SameModule => "same_module",
        RefConfidence::FuzzyName => "fuzzy_name",
    }
}

fn neighbors(
    conn: &Connection,
    symbol: &ImpactSymbol,
    min_confidence: RefConfidence,
) -> Result<Vec<ImpactNeighbor>, GraphError> {
    let mut neighbors = inbound_ref_neighbors(
        conn,
        symbol.qualified.as_str(),
        symbol.name.as_str(),
        min_confidence,
    )?;
    neighbors.extend(outbound_call_neighbors(conn, symbol, min_confidence)?);
    neighbors.extend(relation_neighbors(
        conn,
        symbol.qualified.as_str(),
        min_confidence,
    )?);
    Ok(neighbors)
}

// Source attribution falls back to `r.from_file` when no indexed symbol span
// encloses the call site: `refs` surfaces such edges directly (it only needs the
// call-site file/offset), but `impact` needs a node to attribute the edge to. Without
// the COALESCE these edges would be silently dropped, so `impact` would report an
// empty blast radius for symbols that `refs` reports as referenced (ORB-00381).
//
// The `fuzzy_name` branch mirrors `refs::query_refs`: `fuzzy_name` edges store a NULL
// `target_qualified` and are matchable only by `target_name`, so keying solely on
// `target_qualified` makes every fuzzy edge invisible to `impact`. We only widen to the
// name match when the confidence floor admits fuzzy edges (otherwise they would be
// filtered out anyway).
fn inbound_ref_neighbors(
    conn: &Connection,
    qualified: &str,
    name: &str,
    min_confidence: RefConfidence,
) -> Result<Vec<ImpactNeighbor>, GraphError> {
    const SOURCE_QUALIFIED: &str = "COALESCE(
                (
                    SELECT s.qualified
                    FROM symbols s
                    WHERE s.file_path = r.from_file
                      AND s.span_start <= r.from_span_start
                      AND s.span_end >= r.from_span_end
                    ORDER BY (s.span_end - s.span_start), s.id
                    LIMIT 1
                ),
                r.from_file
            ) AS source_qualified";
    let include_fuzzy_name = min_confidence == RefConfidence::FuzzyName;
    let rows = if include_fuzzy_name {
        let sql = format!(
            "SELECT {SOURCE_QUALIFIED}, r.kind, r.confidence
             FROM refs r
             WHERE r.target_qualified = ?1
                OR (r.confidence = 'fuzzy_name' AND r.target_name = ?2)
             ORDER BY r.from_file, r.from_span_start, r.id"
        );
        let mut stmt = conn
            .prepare_cached(sql.as_str())
            .map_err(|source| GraphError::sqlite("prepare impact inbound refs query", source))?;
        stmt.query_map(params![qualified, name], raw_neighbor_row)
            .map_err(|source| GraphError::sqlite("execute impact inbound refs query", source))?
            .collect::<Result<Vec<_>, _>>()
    } else {
        let sql = format!(
            "SELECT {SOURCE_QUALIFIED}, r.kind, r.confidence
             FROM refs r
             WHERE r.target_qualified = ?1
             ORDER BY r.from_file, r.from_span_start, r.id"
        );
        let mut stmt = conn
            .prepare_cached(sql.as_str())
            .map_err(|source| GraphError::sqlite("prepare impact inbound refs query", source))?;
        stmt.query_map(params![qualified], raw_neighbor_row)
            .map_err(|source| GraphError::sqlite("execute impact inbound refs query", source))?
            .collect::<Result<Vec<_>, _>>()
    }
    .map_err(|source| GraphError::sqlite("collect impact inbound refs rows", source))?;

    rows_to_neighbors(rows, min_confidence)
}

fn raw_neighbor_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawNeighborRow> {
    Ok(RawNeighborRow {
        qualified_name: row.get(0)?,
        kind: row.get(1)?,
        confidence: row.get(2)?,
    })
}

fn outbound_call_neighbors(
    conn: &Connection,
    symbol: &ImpactSymbol,
    min_confidence: RefConfidence,
) -> Result<Vec<ImpactNeighbor>, GraphError> {
    let span = SymbolSpan {
        file_path: symbol.file_path.clone(),
        span_start: symbol.span_start,
        span_end: symbol.span_end,
    };
    let mut neighbors = Vec::new();
    for edge in callees::edges_for_symbol(conn, &span)? {
        if !confidence_visible_at_floor(edge.confidence.as_str(), min_confidence)? {
            continue;
        }
        let Some(qualified_name) = edge.target_qualified else {
            continue;
        };
        neighbors.push(ImpactNeighbor {
            qualified_name,
            edge_kind: RefKind::Call,
        });
    }
    Ok(neighbors)
}

fn relation_neighbors(
    conn: &Connection,
    qualified: &str,
    min_confidence: RefConfidence,
) -> Result<Vec<ImpactNeighbor>, GraphError> {
    let mut neighbors = relation_rows(
        conn,
        "SELECT from_qualified, kind, confidence
         FROM relations
         WHERE to_qualified = ?1
         ORDER BY def_file, def_span_start, id",
        qualified,
        "impact inbound relations",
        min_confidence,
    )?;
    neighbors.extend(relation_rows(
        conn,
        "SELECT to_qualified, kind, confidence
         FROM relations
         WHERE from_qualified = ?1
         ORDER BY def_file, def_span_start, id",
        qualified,
        "impact outbound relations",
        min_confidence,
    )?);
    Ok(neighbors)
}

fn relation_rows(
    conn: &Connection,
    sql: &str,
    qualified: &str,
    operation: &'static str,
    min_confidence: RefConfidence,
) -> Result<Vec<ImpactNeighbor>, GraphError> {
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|source| GraphError::sqlite("prepare relation rows for impact", source))?;
    let rows = stmt
        .query_map(params![qualified], |row| {
            Ok(RawNeighborRow {
                qualified_name: Some(row.get(0)?),
                kind: row.get(1)?,
                confidence: row.get(2)?,
            })
        })
        .map_err(|source| GraphError::sqlite(operation, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| GraphError::sqlite("collect relation rows for impact", source))?;

    rows_to_neighbors(rows, min_confidence)
}

fn rows_to_neighbors(
    rows: Vec<RawNeighborRow>,
    min_confidence: RefConfidence,
) -> Result<Vec<ImpactNeighbor>, GraphError> {
    let mut neighbors = Vec::with_capacity(rows.len());
    for row in rows {
        if !confidence_visible_at_floor(row.confidence.as_str(), min_confidence)? {
            continue;
        }
        let Some(qualified_name) = row.qualified_name else {
            continue;
        };
        neighbors.push(ImpactNeighbor {
            qualified_name,
            edge_kind: RefKind::from_db(row.kind.as_str())?,
        });
    }
    Ok(neighbors)
}

fn confidence_visible_at_floor(
    confidence: &str,
    min_confidence: RefConfidence,
) -> Result<bool, GraphError> {
    Ok(RefConfidence::from_db(confidence)?.visible_at_floor(min_confidence))
}

fn resolve_selector(
    conn: &Connection,
    selector: &Selector,
) -> Result<Option<ImpactSymbol>, GraphError> {
    match selector {
        Selector::Symbol { path, symbol, kind } => {
            resolve_symbol_selector(conn, path, symbol, kind)
        }
        Selector::Module { qualified } => resolve_module_selector(conn, qualified),
        Selector::Command { name } => resolve_command_selector(conn, name),
        Selector::File { .. } | Selector::Dir { .. } => Ok(None),
    }
}

fn resolve_symbol_selector(
    conn: &Connection,
    path: &str,
    symbol: &str,
    kind: &str,
) -> Result<Option<ImpactSymbol>, GraphError> {
    if kind.trim().is_empty() {
        conn.query_row(
            "SELECT file_path, qualified, name, span_start, span_end
             FROM symbols
             WHERE file_path = ?1
               AND (name = ?2 OR qualified = ?2)
             ORDER BY CASE WHEN qualified = ?2 THEN 0 WHEN name = ?2 THEN 1 ELSE 2 END, id
             LIMIT 1",
            params![path, symbol],
            impact_symbol_from_row,
        )
    } else {
        conn.query_row(
            "SELECT file_path, qualified, name, span_start, span_end
             FROM symbols
             WHERE file_path = ?1
               AND kind = ?3
               AND (name = ?2 OR qualified = ?2)
             ORDER BY CASE WHEN qualified = ?2 THEN 0 WHEN name = ?2 THEN 1 ELSE 2 END, id
             LIMIT 1",
            params![path, symbol, kind],
            impact_symbol_from_row,
        )
    }
    .optional()
    .map_err(|source| GraphError::sqlite("resolve impact symbol selector", source))
}

fn resolve_module_selector(
    conn: &Connection,
    qualified: &str,
) -> Result<Option<ImpactSymbol>, GraphError> {
    conn.query_row(
        "SELECT file_path, qualified, name, span_start, span_end
         FROM symbols
         WHERE kind = 'module'
           AND (qualified = ?1 OR name = ?1)
         ORDER BY CASE WHEN qualified = ?1 THEN 0 WHEN name = ?1 THEN 1 ELSE 2 END, id
         LIMIT 1",
        params![qualified],
        impact_symbol_from_row,
    )
    .optional()
    .map_err(|source| GraphError::sqlite("resolve impact module selector", source))
}

fn resolve_command_selector(
    conn: &Connection,
    name: &str,
) -> Result<Option<ImpactSymbol>, GraphError> {
    conn.query_row(
        "SELECT s.file_path, s.qualified, s.name, s.span_start, s.span_end
         FROM commands c
         JOIN symbols s ON s.id = c.handler_symbol
         WHERE c.name = ?1
         ORDER BY c.name
         LIMIT 1",
        params![name],
        impact_symbol_from_row,
    )
    .optional()
    .map_err(|source| GraphError::sqlite("resolve impact command selector", source))
}

fn resolve_symbol_by_qualified(
    conn: &Connection,
    qualified: &str,
) -> Result<Option<ImpactSymbol>, GraphError> {
    conn.query_row(
        "SELECT file_path, qualified, name, span_start, span_end
         FROM symbols
         WHERE qualified = ?1
         ORDER BY id
         LIMIT 1",
        params![qualified],
        impact_symbol_from_row,
    )
    .optional()
    .map_err(|source| GraphError::sqlite("resolve impact frontier symbol", source))
}

fn impact_symbol_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImpactSymbol> {
    Ok(ImpactSymbol {
        file_path: row.get(0)?,
        qualified: row.get(1)?,
        name: row.get(2)?,
        span_start: row.get(3)?,
        span_end: row.get(4)?,
    })
}

#[derive(Debug, Clone)]
struct ImpactSymbol {
    file_path: String,
    qualified: String,
    name: String,
    span_start: i64,
    span_end: i64,
}

#[derive(Debug, Clone)]
struct ImpactNeighbor {
    qualified_name: String,
    edge_kind: RefKind,
}

struct ImpactTraversal {
    touched: Vec<ImpactEntry>,
    truncated: bool,
}

struct RawNeighborRow {
    qualified_name: Option<String>,
    kind: String,
    confidence: String,
}

#[cfg(test)]
#[path = "tests/impact.rs"]
mod tests;
