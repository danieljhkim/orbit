//! Row DTOs and selector helpers for the graph SQLite index.

use std::collections::HashMap;

use rusqlite::Row;

use super::super::nodes::{BaseNodeFields, CodebaseGraphV1, LeafKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphIndexNodeRow {
    pub id: String,
    pub node_type: String,
    pub kind: Option<String>,
    pub location: String,
    pub parent_id: Option<String>,
    pub selector: Option<String>,
    pub object_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphIndexSearchRow {
    pub id: String,
    pub node_type: String,
    pub kind: Option<String>,
    pub name: String,
    pub location: String,
    pub selector: Option<String>,
    pub scan_order: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphIndexReferenceRow {
    pub id: String,
    pub node_type: String,
    pub kind: Option<String>,
    pub name: String,
    pub location: String,
    pub selector: Option<String>,
    pub scan_order: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphIndexCallerRow {
    pub selector: String,
    pub name: String,
    pub file: String,
    pub kind: String,
    pub distance: usize,
    pub via: String,
}

pub(crate) fn graph_index_node_from_row(row: &Row<'_>) -> rusqlite::Result<GraphIndexNodeRow> {
    Ok(GraphIndexNodeRow {
        id: row.get(0)?,
        node_type: row.get(1)?,
        kind: row.get(2)?,
        location: row.get(3)?,
        parent_id: row.get(4)?,
        selector: row.get(5)?,
        object_hash: row.get(6)?,
    })
}

pub(crate) fn graph_index_search_row_from_row(
    row: &Row<'_>,
) -> rusqlite::Result<GraphIndexSearchRow> {
    Ok(GraphIndexSearchRow {
        id: row.get(0)?,
        node_type: row.get(1)?,
        kind: row.get(2)?,
        name: row.get(3)?,
        location: row.get(4)?,
        selector: row.get(5)?,
        scan_order: row.get(6)?,
    })
}

pub(crate) fn graph_index_reference_row_from_row(
    row: &Row<'_>,
) -> rusqlite::Result<GraphIndexReferenceRow> {
    Ok(GraphIndexReferenceRow {
        id: row.get(0)?,
        node_type: row.get(1)?,
        kind: row.get(2)?,
        name: row.get(3)?,
        location: row.get(4)?,
        selector: row.get(5)?,
        scan_order: row.get(6)?,
    })
}

pub(crate) fn sqlite_like_substring_pattern(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len() + 2);
    escaped.push('%');
    for ch in input.chars() {
        match ch {
            '%' | '_' | '\\' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped.push('%');
    escaped
}

pub(crate) fn selector_counts(graph: &CodebaseGraphV1) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for dir in &graph.dirs {
        count_selector(&mut counts, dir_selector(&dir.base));
    }
    for file in &graph.files {
        count_selector(&mut counts, file_selector(&file.base));
    }
    for leaf in &graph.leaves {
        count_selector(&mut counts, leaf_selector(&leaf.base, &leaf.kind));
    }
    counts
}

fn count_selector(counts: &mut HashMap<String, usize>, selector: String) {
    *counts.entry(selector).or_default() += 1;
}

pub(crate) fn stable_selector(selector: String, counts: &HashMap<String, usize>) -> Option<String> {
    (counts.get(&selector) == Some(&1)).then_some(selector)
}

pub(crate) fn dir_selector(base: &BaseNodeFields) -> String {
    let location = base.location.trim_end_matches('/');
    let location = if location.is_empty() { "." } else { location };
    format!("dir:{location}")
}

pub(crate) fn file_selector(base: &BaseNodeFields) -> String {
    format!("file:{}", base.location)
}

pub(crate) fn leaf_selector(base: &BaseNodeFields, kind: &LeafKind) -> String {
    format!("symbol:{}:{}", base.location, kind)
}
