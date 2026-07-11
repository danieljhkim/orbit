// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::sync::RwLock;

use orbit_common::types::OrbitError;

use super::search_index::EnvelopeSnapshot;
use crate::{IdAllocator, Store};

/// Stable registered workspace ID used by test constructors that don't take
/// one explicitly. Direct-index tests must scope their raw index calls to the
/// same value so they observe the rows a `LearningFileStore` writes.
#[cfg(test)]
pub(crate) const TEST_WORKSPACE_ID: &str = "ws-000000";

/// Workspace-scoped, filesystem-backed learning store.
///
/// YAML files at `<root>/<id>/learning.yaml` are the source of truth. Status
/// lives in the YAML body. When `index` is attached, envelope
/// metadata mirrors into the shared SQLite `learnings_index` table for fast
/// scope-glob lookups; the filesystem walk is the fallback path when the
/// index is absent (e.g. tests using `LearningFileStore::new`).
///
/// The shared `learnings_index` table is host-global and holds rows for every
/// workspace bound to the same database, so every index operation is scoped by
/// `workspace_id` — the stable registered Orbit workspace ID (ORB-10113).
/// Without it, a multi-workspace sweep over the shared database would search,
/// truncate, or overwrite another workspace's rows.
///
/// Search is on the hot path (called from injection layers; budget < 10 ms
/// per the design's §5.2). The store keeps an in-memory `envelope_cache`
/// over the active envelope set so 1000 sequential `search` calls don't
/// each pay SQLite lock + JSON-array decode overhead. Cache is invalidated
/// on every mutating call.
pub(crate) struct LearningFileStore {
    pub(super) root: PathBuf,
    pub(super) index: Option<Store>,
    pub(super) id_allocator: IdAllocator,
    /// Stable registered workspace ID that scopes every `index` operation.
    pub(super) workspace_id: String,
    pub(super) envelope_cache: RwLock<Option<std::sync::Arc<Vec<EnvelopeSnapshot>>>>,
}

impl LearningFileStore {
    #[cfg(test)]
    pub(crate) fn new(root: PathBuf) -> Self {
        let id_allocator = IdAllocator::for_test_roots(root.join(".adrs"), root.clone());
        Self {
            root,
            index: None,
            id_allocator,
            workspace_id: TEST_WORKSPACE_ID.to_string(),
            envelope_cache: RwLock::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_index(root: PathBuf, index: Store) -> Self {
        let id_allocator = IdAllocator::for_test_roots(root.join(".adrs"), root.clone());
        Self::new_with_index_and_allocator(root, index, id_allocator, TEST_WORKSPACE_ID.to_string())
    }

    /// Test constructor that pins an explicit workspace ID so multiple stores
    /// can share one SQLite index while staying isolated (ORB-10113).
    #[cfg(test)]
    pub(crate) fn new_with_index_and_workspace(
        root: PathBuf,
        index: Store,
        workspace_id: impl Into<String>,
    ) -> Self {
        let id_allocator = IdAllocator::for_test_roots(root.join(".adrs"), root.clone());
        Self::new_with_index_and_allocator(root, index, id_allocator, workspace_id.into())
    }

    pub(crate) fn new_with_index_and_allocator(
        root: PathBuf,
        index: Store,
        id_allocator: IdAllocator,
        workspace_id: String,
    ) -> Self {
        Self {
            root,
            index: Some(index),
            id_allocator,
            workspace_id,
            envelope_cache: RwLock::new(None),
        }
    }

    pub(crate) fn reject_legacy_flat_layout(root: &std::path::Path) -> Result<(), OrbitError> {
        super::super::migration::reject_legacy_flat_layout(root)
    }
}
