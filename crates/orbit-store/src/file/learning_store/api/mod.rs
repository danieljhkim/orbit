// Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

//! Split of the learning-store API surface (ORB-00116).
//!
//! - `store`: owns `LearningFileStore`, constructors, and legacy-layout reject hook.
//! - `crud`: pure create/get/list/update plus dual-write + cache invalidation.
//! - `lifecycle`: supersede/archive/delete preserving allocation/per-id lock ordering and write/index sequence.
//! - `search_index`: envelope cache, active_envelopes, search/reindex, ranking helpers; `upsert_index_row`/`invalidate_envelope_cache` are `pub(super)` for cross-module mutation paths.
//! - `validation`: reindex-side envelope validation helpers.
//!
//! The public (pub(crate)) surface on `LearningFileStore` is unchanged; `learning_store/mod.rs` continues to re-export it.

mod crud;
mod lifecycle;
mod search_index;
mod store;
mod validation;

#[cfg(test)]
mod tests;

pub(crate) use store::LearningFileStore;
