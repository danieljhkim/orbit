//! Unit tests for `store` (vector SQLite index), split by source file per
//! `docs/design-patterns/test_layout.md` (ORB-00230 sibling migration).
//!
//! The parent `store/mod.rs` declares `#[cfg(test)] mod tests;`.

#![allow(missing_docs)]

mod adrs;
mod docs;
mod learning;
mod queries;
mod schema;
mod tasks;
mod upsert;
