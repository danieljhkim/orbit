//! Unit tests for `object_store`, split by concern per test_layout.md.
//!
//! Declares submodules for read options/hydration, write_graph + sqlite index side effects,
//! and the dir_depth helper (private but visible to submodule tree).

mod dir_depth;
mod read_options;
mod write_graph;
