//! [ORB-10330] F2 composition of hub-preallocated knowledge creation.
//!
//! `orbit.adr.add` and `orbit.learning.add` become composite owner operations
//! on the multi-host path: the singular hub allocates one global id
//! (ORB-10272) and the declared owner finalizes exactly that id in its exact
//! validated checkout (ORB-10262). This module owns the ordering and
//! correlation contract that ties those two halves together.
//!
//! It is deliberately gated inactive for F2: public `orbit.adr.add` /
//! `orbit.learning.add` stay on the standalone compatibility allocator until F3
//! (ORB-10274) atomically disables the legacy authoring paths, enables global
//! issuance, and wires the D3-selected owner-finalize capability to
//! [`orbit_core::OrbitRuntime::finalize_preallocated_adr`] /
//! `finalize_preallocated_learning` plus the correlated owner-finalization
//! audit.

use orbit_common::types::{
    HubKnowledgeAllocationRequestV1, HubKnowledgeAllocationV1, OrbitError, ToolSessionContext,
};

use crate::HubKnowledgeSequenceService;

/// The D3-resolved ownership of the target workspace relative to this host.
///
/// Replica and foreign-spoke ownership are rejected *before* any allocation, so
/// a request that could never finalize locally never consumes a global id and
/// never opens an owner/spoke connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KnowledgeOwnerPlacement {
    /// This hub owns the workspace: allocate here and finalize in the validated
    /// owner checkout the hub controls.
    LocalOwner,
    /// Another spoke owns the workspace: reject before allocation. The hub
    /// never proxies to a spoke owner.
    AnotherSpoke { owner_machine_id: String },
    /// A local replica of a remote owner: reject before allocation. Replicas
    /// are opt-in reads and never author owner-placed knowledge.
    LocalReplica { owner_machine_id: String },
}

impl KnowledgeOwnerPlacement {
    fn require_local_owner(&self, workspace_id: &str) -> Result<(), OrbitError> {
        match self {
            Self::LocalOwner => Ok(()),
            Self::AnotherSpoke { owner_machine_id } => Err(OrbitError::InvalidInput(format!(
                "workspace '{workspace_id}' is owned by another spoke ({owner_machine_id}); its owner authors knowledge — route actionable work to the owner as a task"
            ))),
            Self::LocalReplica { owner_machine_id } => Err(OrbitError::InvalidInput(format!(
                "workspace '{workspace_id}' is a local replica of owner '{owner_machine_id}'; replicas never author owner-placed knowledge"
            ))),
        }
    }
}

/// Compose one hub allocation with one owner-local finalization for a public
/// `orbit.adr.add` / `orbit.learning.add` on the multi-host path.
///
/// Ordering guarantees:
/// 1. D3 owner preflight rejects replica / foreign-spoke ownership before
///    allocation, so no global id is consumed for a request that can never
///    finalize locally.
/// 2. Exactly one immutable hub allocation is drawn (ORB-10272). It stays
///    consumed forever: a finalize failure leaves a valid gap and never
///    abandons, releases, or reuses the id.
/// 3. `finalize` runs in the D3-selected owner checkout with the allocated id;
///    on failure the store finalizer removes only its local partials while the
///    hub allocation stays consumed.
///
/// `finalize` is the D3-selected checkout-bound owner capability. Allocation and
/// finalization correlate through the original trusted `allocation.mcp_call_id`
/// (equal to `context.mcp_call_id`), plus the workspace id, knowledge kind, and
/// allocated id carried on the returned allocation.
pub(crate) fn compose_preallocated_knowledge_add<F>(
    allocator: &HubKnowledgeSequenceService,
    request: &HubKnowledgeAllocationRequestV1,
    context: &ToolSessionContext,
    placement: KnowledgeOwnerPlacement,
    finalize: F,
) -> Result<HubKnowledgeAllocationV1, OrbitError>
where
    F: FnOnce(&HubKnowledgeAllocationV1) -> Result<(), OrbitError>,
{
    // D3 preflight: reject before allocation so no avoidable gap is consumed.
    placement.require_local_owner(&request.workspace_id)?;

    // One immutable global allocation. Consumed forever — never rolled back.
    let allocation = allocator.allocate(request, context)?;

    // Finalize in the exact owner checkout with the allocated id. A failure
    // here does not release the allocation; the id stays consumed as a gap.
    finalize(&allocation)?;

    Ok(allocation)
}
