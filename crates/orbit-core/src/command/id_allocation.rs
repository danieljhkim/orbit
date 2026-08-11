//! [ORB-10501] Detection and repair for id-allocation rows whose pinned
//! worktree no longer exists.
//!
//! Learning ids are allocated from a workspace-local SQLite allocator and
//! pinned to the worktree that allocated them (`docs/design/worktree-artifacts/`).
//! When that worktree is reaped before the body is finalized and merged, the
//! allocation row outlives every path that could resolve it: the body is
//! unrecoverable, the row stays visible as a `reserved`/`merged` remote stub
//! forever, and nothing prunes it. `orbit doctor` reports these rows and
//! `orbit doctor --fix-orphaned-allocations` retires them.
//!
//! The learning store owns the orphan test: a missing worktree and unreadable
//! body must both hold before an allocation can be retired.

use orbit_common::types::OrbitError;
use orbit_store::{IdAllocationKind, IdAllocationRecord};

use crate::OrbitRuntime;

impl OrbitRuntime {
    /// Every learning allocation that can never resolve to a body again.
    pub fn list_orphaned_id_allocations(&self) -> Result<Vec<IdAllocationRecord>, OrbitError> {
        let mut orphaned = self
            .stores()
            .learnings()
            .list_orphaned_learning_allocations()?;
        orphaned.sort_by(|left, right| left.id.cmp(&right.id));
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
                // Historical ADR allocator rows may remain in existing
                // databases, but the retired store has no repair surface.
                IdAllocationKind::Adr => false,
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
