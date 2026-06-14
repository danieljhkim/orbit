//! Trait implementor query.
//!
//! Resolves the concrete types that implement a trait, reading the structural
//! `relations` table (`kind IN ('impl', 'implements')`). The trait reference is
//! stored at the impl site as written (e.g. `AuditSink` or `std::fmt::Display`),
//! so matching is by trailing path segment — passing `Display` matches
//! `std::fmt::Display`. This mirrors v1's "trailing identifiers match" behavior.
//! See ORB-00389.

use orbit_graph_extract::Selector;
use rusqlite::{Connection, params};

use crate::{Graph, GraphError, Implementor, ImplementorsResult, RefKind};

pub(crate) fn run(graph: &Graph, sel: &Selector) -> Result<ImplementorsResult, GraphError> {
    let trait_name = trait_name_from_selector(sel);
    graph.with_read_connection(|conn| {
        let Some(name) = trait_name else {
            return Ok(ImplementorsResult {
                trait_name: sel.to_string(),
                implementors: Vec::new(),
            });
        };
        let implementors = query_implementors(conn, &name)?;
        Ok(ImplementorsResult {
            trait_name: name,
            implementors,
        })
    })
}

/// Derive the trait's short name from the selector. Only symbol and module
/// selectors address a trait; other forms have no trait identity.
fn trait_name_from_selector(sel: &Selector) -> Option<String> {
    let raw = match sel {
        Selector::Symbol { symbol, .. } => symbol.as_str(),
        Selector::Module { qualified } => qualified.as_str(),
        Selector::Dir { .. } | Selector::File { .. } | Selector::Command { .. } => return None,
    };
    let name = trailing_segment(raw);
    if name.is_empty() { None } else { Some(name) }
}

/// Last `::`-separated segment, trimmed. `"std::fmt::Display"` → `"Display"`.
fn trailing_segment(name: &str) -> String {
    name.rsplit("::").next().unwrap_or(name).trim().to_string()
}

fn query_implementors(conn: &Connection, name: &str) -> Result<Vec<Implementor>, GraphError> {
    let trailing = format!("%::{name}");
    let mut stmt = conn
        .prepare_cached(
            "SELECT from_qualified, to_qualified, kind, def_file
             FROM relations
             WHERE kind IN ('impl', 'implements')
               AND (to_qualified = ?1 OR to_qualified LIKE ?2)
             ORDER BY def_file, from_qualified, id",
        )
        .map_err(|source| GraphError::sqlite("prepare implementors lookup", source))?;
    let rows = stmt
        .query_map(params![name, trailing], |row| {
            Ok(StoredImplRow {
                from_qualified: row.get(0)?,
                to_qualified: row.get(1)?,
                kind: row.get(2)?,
                def_file: row.get(3)?,
            })
        })
        .map_err(|source| GraphError::sqlite("query implementors by trait", source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| GraphError::sqlite("collect implementor rows", source))?;

    let mut implementors = Vec::with_capacity(rows.len());
    for row in rows {
        implementors.push(Implementor {
            type_qualified: row.from_qualified,
            trait_matched: row.to_qualified,
            kind: RefKind::from_db(row.kind.as_str())?,
            file: row.def_file,
        });
    }
    Ok(implementors)
}

struct StoredImplRow {
    from_qualified: String,
    to_qualified: String,
    kind: String,
    def_file: String,
}

#[cfg(test)]
#[path = "tests/implementors.rs"]
mod tests;
