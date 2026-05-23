// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

//! Split of the learning-store API surface (ORB-00116).
//!
//! - `store`: owns `LearningFileStore`, constructors, and legacy-layout reject hook.
//! - `crud`: pure create/get/list/update plus dual-write + cache invalidation.
//! - `lifecycle`: supersede/archive/delete preserving allocation/per-id lock ordering and write/index sequence.
//! - `vote_ops`: upvote paths (with test-only time injection) and summary; delegates JSONL to `super::super::votes`.
//! - `comment_ops`: comment add/list/delete/find; uses `super::super::record` + allocation locks.
//! - `search_index`: envelope cache, active_envelopes, search/reindex, ranking helpers; `upsert_index_row`/`invalidate_envelope_cache` are `pub(super)` for cross-module mutation paths.
//! - `validation`: comment body/model/file validators, `pub(super)` so comment and reindex paths can share them.
//!
//! The public (pub(crate)) surface on `LearningFileStore` is unchanged; `learning_store/mod.rs` continues to re-export it.

mod comment_ops;
mod crud;
mod lifecycle;
mod search_index;
mod store;
mod validation;
mod vote_ops;

#[cfg(test)]
mod tests;

pub(crate) use store::LearningFileStore;
