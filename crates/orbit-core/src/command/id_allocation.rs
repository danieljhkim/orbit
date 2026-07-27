//! [ORB-10501] Detection and repair for id-allocation rows whose pinned
//! worktree no longer exists.
//!
//! Learning and ADR ids are allocated from one shared SQLite allocator and
//! pinned to the worktree that allocated them (`docs/design/worktree-artifacts/`).
//! When that worktree is reaped before the body is finalized and merged, the
//! allocation row outlives every path that could resolve it: the body is
//! unrecoverable, the row stays visible as a `reserved`/`merged` remote stub
//! forever, and nothing prunes it. `orbit doctor` reports these rows and
//! `orbit doctor --fix-orphaned-allocations` retires them.
//!
//! The stores own the orphan test — both kinds require a missing worktree
//! *and* an unreadable body — so this surface only fans out over the two
//! kinds and keeps the ordering stable.

use orbit_common::types::OrbitError;
use orbit_store::{IdAllocationKind, IdAllocationRecord};

use crate::OrbitRuntime;

impl OrbitRuntime {
    /// Every learning and ADR allocation that can never resolve to a body
    /// again, ordered by kind then id.
    pub fn list_orphaned_id_allocations(&self) -> Result<Vec<IdAllocationRecord>, OrbitError> {
        let mut orphaned = self.stores().adrs().list_orphaned_adr_allocations()?;
        orphaned.extend(
            self.stores()
                .learnings()
                .list_orphaned_learning_allocations()?,
        );
        orphaned.sort_by(|left, right| {
            left.kind
                .as_str()
                .cmp(right.kind.as_str())
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(orphaned)
    }

    /// Abandon every currently-orphaned allocation, returning the rows that
    /// were retired.
    ///
    /// Each row is re-verified by the owning store immediately before its
    /// write, so an allocation that stopped qualifying between the scan and
    /// the repair (a worktree re-created, a body synced back) is refused
    /// rather than silently retired. A refusal fails the whole call: the
    /// caller asked to clear a set it had just been shown, and a changed set
    /// deserves a fresh look rather than a partial sweep.
    pub fn abandon_orphaned_id_allocations(&self) -> Result<Vec<IdAllocationRecord>, OrbitError> {
        let mut abandoned = Vec::new();
        for record in self.list_orphaned_id_allocations()? {
            let cleared = match record.kind {
                IdAllocationKind::Adr => self
                    .stores()
                    .adrs()
                    .abandon_orphaned_adr_allocation(&record.id)?,
                IdAllocationKind::Learning => self
                    .stores()
                    .learnings()
                    .abandon_orphaned_learning_allocation(&record.id)?,
            };
            if cleared {
                abandoned.push(record);
            }
        }
        Ok(abandoned)
    }
}
