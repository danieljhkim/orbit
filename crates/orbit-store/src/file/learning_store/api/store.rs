// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::sync::RwLock;

use orbit_common::types::OrbitError;

use super::search_index::EnvelopeSnapshot;
use crate::Store;

/// Workspace-scoped, filesystem-backed learning store.
///
/// YAML files at `<root>/<id>/learning.yaml` are the source of truth. Status
/// lives in the YAML body. When `index` is attached, envelope
/// metadata mirrors into the shared SQLite `learnings_index` table for fast
/// scope-glob lookups; the filesystem walk is the fallback path when the
/// index is absent (e.g. tests using `LearningFileStore::new`).
///
/// Search is on the hot path (called from injection layers; budget < 10 ms
/// per the design's §5.2). The store keeps an in-memory `envelope_cache`
/// over the active envelope set so 1000 sequential `search` calls don't
/// each pay SQLite lock + JSON-array decode overhead. Cache is invalidated
/// on every mutating call.
pub(crate) struct LearningFileStore {
    pub(super) root: PathBuf,
    pub(super) index: Option<Store>,
    pub(super) envelope_cache: RwLock<Option<std::sync::Arc<Vec<EnvelopeSnapshot>>>>,
}

impl LearningFileStore {
    #[cfg(test)]
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            index: None,
            envelope_cache: RwLock::new(None),
        }
    }

    pub(crate) fn new_with_index(root: PathBuf, index: Store) -> Self {
        Self {
            root,
            index: Some(index),
            envelope_cache: RwLock::new(None),
        }
    }

    pub(crate) fn reject_legacy_flat_layout(root: &std::path::Path) -> Result<(), OrbitError> {
        super::super::migration::reject_legacy_flat_layout(root)
    }
}
