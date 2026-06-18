//! Outbound module / import edge query.
//!
//! v2 `deps` reports source-level import edges (the `imports` table) for the
//! files addressed by a `file:` or `dir:` selector. This intentionally differs
//! from v1 `orbit.graph.deps`, which reported Cargo workspace crate edges; the
//! v2 graph models module/use edges, not the Cargo dependency graph. See
//! ORB-00389 and GRAPH_SPEC §6.2 (`imports`).

use orbit_graph_extract::Selector;
use rusqlite::{Connection, params};

use crate::{DepEdge, DepsResult, Graph, GraphError};

pub(crate) fn run(graph: &Graph, sel: &Selector) -> Result<DepsResult, GraphError> {
    let scope = ImportScope::from_selector(sel)?;
    graph.with_read_connection(|conn| {
        Ok(DepsResult {
            scope: sel.to_string(),
            imports: query_imports(conn, &scope)?,
        })
    })
}

enum ImportScope {
    /// Imports declared by exactly this file.
    FileEq(String),
    /// Imports declared by any file under this directory prefix.
    DirLike(String),
}

impl ImportScope {
    fn from_selector(sel: &Selector) -> Result<Self, GraphError> {
        match sel {
            Selector::File { path } => Ok(Self::FileEq(path.clone())),
            Selector::Dir { path } => {
                Ok(Self::DirLike(format!("{}/%", path.trim_end_matches('/'))))
            }
            _ => Err(GraphError::invalid_data(
                "resolve deps selector",
                "deps scope must be a `file:` or `dir:` selector",
            )),
        }
    }
}

fn query_imports(conn: &Connection, scope: &ImportScope) -> Result<Vec<DepEdge>, GraphError> {
    let (predicate, param) = match scope {
        ImportScope::FileEq(path) => ("from_file = ?1", path.as_str()),
        ImportScope::DirLike(pattern) => ("from_file LIKE ?1", pattern.as_str()),
    };
    let sql = format!(
        "SELECT from_file, target_path, target_symbol
         FROM imports
         WHERE {predicate}
         ORDER BY from_file, target_path, target_symbol"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|source| GraphError::sqlite("prepare deps lookup", source))?;
    let rows = stmt
        .query_map(params![param], |row| {
            Ok(DepEdge {
                from_file: row.get(0)?,
                target_path: row.get(1)?,
                target_symbol: row.get(2)?,
            })
        })
        .map_err(|source| GraphError::sqlite("query deps imports", source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| GraphError::sqlite("collect deps import rows", source))?;
    Ok(rows)
}

#[cfg(test)]
#[path = "tests/deps.rs"]
mod tests;
