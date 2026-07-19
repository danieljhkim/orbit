//! [ORB-10330] Tests for the gated F2 knowledge-add composition.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use orbit_common::types::{
    HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION, HubKnowledgeAllocationRequestV1, KnowledgeIdKind,
    McpCapability, McpTransport, OrbitError, ToolSessionContext,
};

use crate::knowledge_broker::{KnowledgeOwnerPlacement, compose_preallocated_knowledge_add};
use crate::persistence::KnowledgeWorkspaceInventory;
use crate::{HubKnowledgeSequenceService, RemoteStore};

fn active_service(workspaces: &[&str]) -> HubKnowledgeSequenceService {
    let store = RemoteStore::open_in_memory().expect("store");
    let service = HubKnowledgeSequenceService::new_for_test(
        store,
        workspaces.iter().map(|w| (*w).to_string()).collect(),
    );
    service
        .activate(
            workspaces
                .iter()
                .map(|workspace_id| KnowledgeWorkspaceInventory {
                    workspace_id: (*workspace_id).to_string(),
                    ids: Vec::new(),
                })
                .collect(),
        )
        .expect("activate allocator");
    service
}

fn request(workspace_id: &str, kind: KnowledgeIdKind) -> HubKnowledgeAllocationRequestV1 {
    HubKnowledgeAllocationRequestV1 {
        schema_version: HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION,
        workspace_id: workspace_id.to_string(),
        kind,
        model: Some("gpt-test".to_string()),
    }
}

fn context(workspace_id: &str, call_id: &str) -> ToolSessionContext {
    ToolSessionContext {
        workspace: None,
        workspace_id: Some(workspace_id.to_string()),
        caller_machine_id: Some("hm_spoke".to_string()),
        caller_host_id: Some("spoke".to_string()),
        process_machine_id: Some("hm_hub".to_string()),
        process_host_id: Some("hub".to_string()),
        transport: Some(McpTransport::SshMcp),
        effective_capabilities: BTreeSet::from([McpCapability::Agent]),
        origin_session_id: Some("session-broker".to_string()),
        mcp_call_id: Some(call_id.to_string()),
        leased_run: None,
    }
}

#[test]
fn local_owner_allocates_once_then_finalizes_the_same_id() {
    let service = active_service(&["ws_alpha"]);
    let finalized: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let sink = Arc::clone(&finalized);
    let allocation = compose_preallocated_knowledge_add(
        &service,
        &request("ws_alpha", KnowledgeIdKind::Learning),
        &context("ws_alpha", "mcall-broker-1"),
        KnowledgeOwnerPlacement::LocalOwner,
        move |allocation| {
            sink.lock().expect("lock").push(allocation.id.clone());
            Ok(())
        },
    )
    .expect("compose succeeds");

    assert_eq!(allocation.kind, KnowledgeIdKind::Learning);
    assert_eq!(allocation.mcp_call_id, "mcall-broker-1");
    // The finalizer ran exactly once, with the id the hub chose.
    assert_eq!(
        &*finalized.lock().expect("lock"),
        std::slice::from_ref(&allocation.id)
    );

    // Allocation and finalization correlate by mcp_call_id — exactly one
    // consumed allocation is observable.
    let by_call = service
        .allocation_by_call("mcall-broker-1")
        .expect("lookup")
        .expect("allocation exists");
    assert_eq!(by_call, allocation);
}

#[test]
fn another_spoke_owner_is_rejected_before_allocation() {
    let service = active_service(&["ws_alpha"]);
    let called = Arc::new(Mutex::new(false));

    let flag = Arc::clone(&called);
    let err = compose_preallocated_knowledge_add(
        &service,
        &request("ws_alpha", KnowledgeIdKind::Adr),
        &context("ws_alpha", "mcall-broker-foreign"),
        KnowledgeOwnerPlacement::AnotherSpoke {
            owner_machine_id: "hm_other".to_string(),
        },
        move |_| {
            *flag.lock().expect("lock") = true;
            Ok(())
        },
    )
    .expect_err("foreign owner must be rejected");
    assert!(matches!(err, OrbitError::InvalidInput(_)), "got {err:?}");

    // The finalize capability was never invoked and no allocation was consumed.
    assert!(!*called.lock().expect("lock"));
    assert!(
        service
            .allocation_by_call("mcall-broker-foreign")
            .expect("lookup")
            .is_none()
    );
    // The next real allocation still gets the first sequence: no gap was burned.
    let allocation = service
        .allocate(
            &request("ws_alpha", KnowledgeIdKind::Adr),
            &context("ws_alpha", "mcall-broker-after"),
        )
        .expect("allocate after rejection");
    assert_eq!(allocation.sequence, 1);
}

#[test]
fn local_replica_is_rejected_before_allocation() {
    let service = active_service(&["ws_alpha"]);
    let err = compose_preallocated_knowledge_add(
        &service,
        &request("ws_alpha", KnowledgeIdKind::Learning),
        &context("ws_alpha", "mcall-broker-replica"),
        KnowledgeOwnerPlacement::LocalReplica {
            owner_machine_id: "hm_owner".to_string(),
        },
        |_| Ok(()),
    )
    .expect_err("replica must be rejected");
    assert!(matches!(err, OrbitError::InvalidInput(_)), "got {err:?}");
    assert!(
        service
            .allocation_by_call("mcall-broker-replica")
            .expect("lookup")
            .is_none()
    );
}

#[test]
fn finalize_failure_leaves_the_allocation_consumed_as_a_valid_gap() {
    let service = active_service(&["ws_alpha"]);

    let err = compose_preallocated_knowledge_add(
        &service,
        &request("ws_alpha", KnowledgeIdKind::Learning),
        &context("ws_alpha", "mcall-broker-fail"),
        KnowledgeOwnerPlacement::LocalOwner,
        |_| {
            Err(OrbitError::Store(
                "injected owner finalize failure".to_string(),
            ))
        },
    )
    .expect_err("finalize failure propagates");
    assert!(matches!(err, OrbitError::Store(_)), "got {err:?}");

    // The immutable hub allocation stays consumed (never abandoned/released).
    let consumed = service
        .allocation_by_call("mcall-broker-fail")
        .expect("lookup")
        .expect("allocation still consumed");
    assert_eq!(consumed.sequence, 1);

    // The next allocation is strictly higher — the failed id is a valid gap,
    // never reissued.
    let next = service
        .allocate(
            &request("ws_alpha", KnowledgeIdKind::Learning),
            &context("ws_alpha", "mcall-broker-next"),
        )
        .expect("allocate after gap");
    assert_eq!(next.sequence, 2);
}
