//! SQLite-backed secondary index for the knowledge graph, split by responsibility.
//!
//! `rows` defines the two public row DTOs and the selector-keying helpers used for stable dedup.
//! `reader` implements `GraphIndexReader`, the read-only facade and all overview/search/children/lineage queries consumed by the CLI.
//! `writer` owns `write_graph_index`, the SQLite DDL, WAL setup, and the three-phase ingestion (nodes → children → file_summary).
//! `equivalence_tests` (test-only) contains the language-diverse, nested-leaf cross-validation battery.

mod reader;
mod rows;
mod writer;

#[cfg(test)]
mod equivalence_tests;

pub use reader::GraphIndexReader;
pub use rows::{GraphIndexNodeRow, GraphIndexSearchRow};
pub(crate) use writer::write_graph_index;

pub(crate) const GRAPH_SQLITE_INDEX_SCHEMA_VERSION: u32 = 6;
pub(crate) const GRAPH_SQLITE_INDEX_FILENAME: &str = "graph_index.sqlite";
