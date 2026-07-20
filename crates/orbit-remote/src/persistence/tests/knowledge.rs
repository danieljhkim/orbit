use std::collections::{BTreeSet, HashSet};

use orbit_common::types::{
    HUB_KNOWLEDGE_ALLOCATION_METHOD_V1, HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION,
    HubKnowledgeAllocationRequestV1, KnowledgeIdKind, McpCapability, McpLeasedRun, McpTransport,
    OrbitError, ToolSessionContext,
};
use rusqlite::params;

use crate::HubKnowledgeSequenceService;

use super::super::{
    HubKnowledgeAllocatorStatus, KnowledgeAuthorityCutoverStatus, KnowledgeWorkspaceInventory,
    LegacyKnowledgeId, RemoteStore,
};

#[test]
fn authority_cutover_state_is_restart_safe_and_forward_only() {
    let store = RemoteStore::open_in_memory().expect("store");
    let first = store.begin_knowledge_cutover().expect("begin cutover");
    assert_eq!(first.status, KnowledgeAuthorityCutoverStatus::Reconciling);
    assert_eq!(first.generation, 1);

    let failed = store
        .fail_knowledge_cutover(&OrbitError::Migration("source unavailable".into()))
        .expect("record incomplete cutover");
    assert_eq!(
        failed.status,
        KnowledgeAuthorityCutoverStatus::FailedIncomplete
    );
    assert_eq!(
        failed.last_error.as_deref(),
        Some("schema migration failed: source unavailable")
    );

    let resumed = store.begin_knowledge_cutover().expect("resume cutover");
    assert_eq!(resumed.status, KnowledgeAuthorityCutoverStatus::Reconciling);
    assert_eq!(resumed.generation, 2);
    store
        .activate_knowledge_allocator(vec![inventory("ws_alpha", Vec::new())])
        .expect("activate allocator");
    let active = store
        .complete_knowledge_cutover()
        .expect("complete cutover");
    assert_eq!(active.status, KnowledgeAuthorityCutoverStatus::Active);

    assert_eq!(
        store.begin_knowledge_cutover().expect("active is terminal"),
        active
    );
}

fn evidence(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn legacy(kind: KnowledgeIdKind, id: &str, sources: &[&str]) -> LegacyKnowledgeId {
    LegacyKnowledgeId {
        kind,
        id: id.to_string(),
        evidence: evidence(sources),
    }
}

fn inventory(workspace_id: &str, ids: Vec<LegacyKnowledgeId>) -> KnowledgeWorkspaceInventory {
    KnowledgeWorkspaceInventory {
        workspace_id: workspace_id.to_string(),
        ids,
    }
}

fn service(store: RemoteStore, workspaces: &[&str]) -> HubKnowledgeSequenceService {
    HubKnowledgeSequenceService::new_for_test(
        store,
        workspaces
            .iter()
            .map(|workspace| (*workspace).to_string())
            .collect(),
    )
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
        origin_session_id: Some("session-knowledge".to_string()),
        mcp_call_id: Some(call_id.to_string()),
        leased_run: Some(McpLeasedRun {
            run_id: "jrun-knowledge".to_string(),
            lease_id: "lease-knowledge".to_string(),
        }),
    }
}

#[test]
fn two_workspace_activation_seeds_above_every_legacy_max_and_allocates_with_atomic_audit() {
    let store = RemoteStore::open_in_memory().expect("store");
    let service = service(store.clone(), &["ws_alpha", "ws_beta"]);
    let state = service
        .activate(vec![
            inventory(
                "ws_alpha",
                vec![
                    legacy(KnowledgeIdKind::Adr, "ADR-0004", &["adr-file:accepted"]),
                    legacy(
                        KnowledgeIdKind::Learning,
                        "L-0009",
                        &["allocation:reserved"],
                    ),
                ],
            ),
            inventory(
                "ws_beta",
                vec![
                    legacy(KnowledgeIdKind::Adr, "ADR-0012", &["adr-file:deleted"]),
                    legacy(
                        KnowledgeIdKind::Learning,
                        "L-0003",
                        &["allocation:abandoned"],
                    ),
                ],
            ),
        ])
        .expect("activate");
    assert_eq!(state.status, HubKnowledgeAllocatorStatus::Active);
    assert_eq!(state.adr_next_sequence, 13);
    assert_eq!(state.learning_next_sequence, 10);

    let adr_request = request("ws_alpha", KnowledgeIdKind::Adr);
    let adr_context = context("ws_alpha", "mcall-adr-13");
    let adr = service
        .allocate(&adr_request, &adr_context)
        .expect("allocate adr");
    assert_eq!(adr.id, "ADR-0013");
    assert_eq!(adr.sequence, 13);

    let learning = service
        .allocate(
            &request("ws_beta", KnowledgeIdKind::Learning),
            &context("ws_beta", "mcall-learning-10"),
        )
        .expect("allocate learning");
    assert_eq!(learning.id, "L-0010");
    assert_eq!(
        service
            .allocation_by_call("mcall-adr-13")
            .expect("call lookup"),
        Some(adr.clone())
    );
    assert_eq!(
        service
            .allocation_by_id("ws_alpha", KnowledgeIdKind::Adr, "ADR-0013")
            .expect("id lookup"),
        Some(adr.clone())
    );

    let replay = service
        .allocate(&adr_request, &adr_context)
        .expect("exact replay");
    assert_eq!(replay, adr);
    let mut drifted = adr_request;
    drifted.model = Some("different-model".to_string());
    let error = service
        .allocate(&drifted, &adr_context)
        .expect_err("identity drift must fail")
        .to_string();
    assert!(error.contains("already used"), "{error}");

    let connection = store.connection();
    let connection = connection.lock().expect("connection");
    let audit: (String, String, String, String, String, String, String) = connection
        .query_row(
            "SELECT tool_name, target_id, workspace_id, caller_machine_id,
                    process_machine_id, mcp_call_id, lease_id
             FROM audit_events WHERE mcp_call_id = 'mcall-adr-13'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .expect("allocation audit");
    assert_eq!(audit.0, HUB_KNOWLEDGE_ALLOCATION_METHOD_V1);
    assert_eq!(audit.1, "ADR-0013");
    assert_eq!(audit.2, "ws_alpha");
    assert_eq!(audit.3, "hm_spoke");
    assert_eq!(audit.4, "hm_hub");
    assert_eq!(audit.5, "mcall-adr-13");
    assert_eq!(audit.6, "lease-knowledge");
}

#[test]
fn activation_reports_every_cross_workspace_duplicate_before_mutation() {
    let store = RemoteStore::open_in_memory().expect("store");
    let service = service(store.clone(), &["ws_a", "ws_b", "ws_c"]);
    let error = service
        .activate(vec![
            inventory(
                "ws_a",
                vec![
                    legacy(KnowledgeIdKind::Adr, "ADR-0007", &["adr-file:accepted"]),
                    legacy(
                        KnowledgeIdKind::Learning,
                        "L-0002",
                        &["learning-file:active"],
                    ),
                ],
            ),
            inventory(
                "ws_b",
                vec![
                    legacy(KnowledgeIdKind::Adr, "ADR-0007", &["allocation:merged"]),
                    legacy(
                        KnowledgeIdKind::Learning,
                        "L-0002",
                        &["allocation:reserved"],
                    ),
                ],
            ),
            inventory(
                "ws_c",
                vec![legacy(
                    KnowledgeIdKind::Adr,
                    "ADR-0007",
                    &["adr-file:deleted"],
                )],
            ),
        ])
        .expect_err("duplicates must block activation")
        .to_string();
    for expected in [
        "ADR-0007",
        "L-0002",
        "ws_a",
        "ws_b",
        "ws_c",
        "adr-file:accepted",
        "allocation:merged",
        "adr-file:deleted",
    ] {
        assert!(error.contains(expected), "missing '{expected}' in {error}");
    }
    let state = store.knowledge_allocator_state().expect("state");
    assert_eq!(state.status, HubKnowledgeAllocatorStatus::Dormant);
    let count: i64 = store
        .connection()
        .lock()
        .expect("connection")
        .query_row("SELECT COUNT(*) FROM hub_knowledge_ids", [], |row| {
            row.get(0)
        })
        .expect("id count");
    assert_eq!(count, 0);
}

#[test]
fn activation_reports_numeric_normalization_collisions_before_mutation() {
    let store = RemoteStore::open_in_memory().expect("store");
    let service = service(store.clone(), &["ws_a", "ws_b", "ws_c"]);
    let error = service
        .activate(vec![
            inventory(
                "ws_a",
                vec![legacy(
                    KnowledgeIdKind::Adr,
                    "ADR-0001",
                    &["adr-file:accepted"],
                )],
            ),
            inventory(
                "ws_b",
                vec![legacy(
                    KnowledgeIdKind::Adr,
                    "ADR-00001",
                    &["allocation:merged"],
                )],
            ),
            inventory(
                "ws_c",
                vec![legacy(
                    KnowledgeIdKind::Adr,
                    "ADR-000001",
                    &["adr-file:deleted"],
                )],
            ),
        ])
        .expect_err("numeric duplicate must block activation")
        .to_string();
    for expected in [
        "sequence 1",
        "ADR-0001",
        "ADR-00001",
        "ADR-000001",
        "ws_a",
        "ws_b",
        "ws_c",
        "adr-file:accepted",
        "allocation:merged",
        "adr-file:deleted",
    ] {
        assert!(error.contains(expected), "missing '{expected}' in {error}");
    }
    let state = store.knowledge_allocator_state().expect("state");
    assert_eq!(state.status, HubKnowledgeAllocatorStatus::Dormant);
    let count: i64 = store
        .connection()
        .lock()
        .expect("connection")
        .query_row("SELECT COUNT(*) FROM hub_knowledge_ids", [], |row| {
            row.get(0)
        })
        .expect("id count");
    assert_eq!(count, 0);
}

#[test]
fn activation_requires_the_exact_nonempty_registered_workspace_inventory_set() {
    let store = RemoteStore::open_in_memory().expect("store");
    let allocator = service(store.clone(), &["ws_alpha", "ws_beta"]);
    let error = allocator
        .activate(vec![inventory("ws_alpha", Vec::new())])
        .expect_err("partial inventory must fail")
        .to_string();
    assert!(error.contains("ws_beta"), "{error}");
    assert!(error.contains("missing"), "{error}");
    let error = allocator
        .activate(Vec::new())
        .expect_err("empty inventory must fail")
        .to_string();
    assert!(error.contains("ws_alpha"), "{error}");
    assert!(error.contains("ws_beta"), "{error}");
    assert_eq!(
        store.knowledge_allocator_state().expect("state").status,
        HubKnowledgeAllocatorStatus::Dormant
    );

    let empty_registry_store = RemoteStore::open_in_memory().expect("empty registry store");
    let empty_registry_service = service(empty_registry_store.clone(), &[]);
    let error = empty_registry_service
        .activate(Vec::new())
        .expect_err("empty registered-workspace set still cannot establish authority")
        .to_string();
    assert!(error.contains("exactly"), "{error}");
    assert_eq!(
        empty_registry_store
            .knowledge_allocator_state()
            .expect("empty registry state")
            .status,
        HubKnowledgeAllocatorStatus::Dormant
    );
}

#[test]
fn audit_failure_rolls_back_sequence_occupancy_and_ledger() {
    let store = RemoteStore::open_in_memory().expect("store");
    let service = service(store.clone(), &["ws_alpha"]);
    service
        .activate(vec![inventory("ws_alpha", Vec::new())])
        .expect("activate");
    store
        .connection()
        .lock()
        .expect("connection")
        .execute_batch(
            "CREATE TRIGGER fail_knowledge_allocation_audit
             BEFORE INSERT ON audit_events
             WHEN NEW.tool_name = 'orbit/private/allocate-knowledge-id/v1'
             BEGIN
                 SELECT RAISE(ABORT, 'injected audit failure');
             END;",
        )
        .expect("failure trigger");

    let request = request("ws_alpha", KnowledgeIdKind::Adr);
    let context = context("ws_alpha", "mcall-rollback");
    let error = service
        .allocate(&request, &context)
        .expect_err("audit failure")
        .to_string();
    assert!(error.contains("injected audit failure"), "{error}");
    let connection = store.connection();
    let connection = connection.lock().expect("connection");
    let (next, ids, ledger, audits): (i64, i64, i64, i64) = connection
        .query_row(
            "SELECT
                (SELECT next_sequence FROM hub_knowledge_sequences WHERE kind = 'adr'),
                (SELECT COUNT(*) FROM hub_knowledge_ids),
                (SELECT COUNT(*) FROM hub_knowledge_allocation_ledger),
                (SELECT COUNT(*) FROM audit_events WHERE mcp_call_id = 'mcall-rollback')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("rollback state");
    assert_eq!((next, ids, ledger, audits), (1, 0, 0, 0));
    connection
        .execute_batch("DROP TRIGGER fail_knowledge_allocation_audit")
        .expect("remove trigger");
    drop(connection);

    let allocation = service
        .allocate(&request, &context)
        .expect("retry after definitive rollback");
    assert_eq!(allocation.id, "ADR-0001");
}

#[test]
fn activation_authority_update_failure_rolls_back_inventory_sequences_and_reconciliation() {
    let directory = tempfile::tempdir().expect("tempdir");
    let database = directory.path().join("remote.db");
    let store = RemoteStore::open(&database).expect("store");
    store
        .connection()
        .lock()
        .expect("connection")
        .execute_batch(
            "CREATE TRIGGER fail_knowledge_activation_authority
             BEFORE UPDATE ON hub_knowledge_allocator_state
             WHEN NEW.status = 'active'
             BEGIN
                 SELECT RAISE(ABORT, 'injected activation authority failure');
             END;",
        )
        .expect("failure trigger");
    let service = service(store.clone(), &["ws_alpha"]);
    let error = service
        .activate(vec![inventory(
            "ws_alpha",
            vec![
                legacy(KnowledgeIdKind::Adr, "ADR-0042", &["adr-file:accepted"]),
                legacy(
                    KnowledgeIdKind::Learning,
                    "L-0017",
                    &["allocation:reserved"],
                ),
            ],
        )])
        .expect_err("activation authority failure")
        .to_string();
    assert!(
        error.contains("injected activation authority failure"),
        "{error}"
    );
    drop(service);
    drop(store);

    let reopened = RemoteStore::open(&database).expect("reopen");
    let state = reopened.knowledge_allocator_state().expect("state");
    assert_eq!(state.status, HubKnowledgeAllocatorStatus::Dormant);
    assert_eq!(state.activation_generation, 0);
    assert_eq!(state.adr_next_sequence, 1);
    assert_eq!(state.learning_next_sequence, 1);
    let connection = reopened.connection();
    let connection = connection.lock().expect("connection");
    let (occupancy, reconciliations): (i64, i64) = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM hub_knowledge_ids),
                (SELECT COUNT(*) FROM hub_knowledge_workspace_reconciliation)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("rollback rows");
    assert_eq!((occupancy, reconciliations), (0, 0));
}

#[test]
fn concurrent_file_backed_allocations_are_unique_and_monotonic() {
    let directory = tempfile::tempdir().expect("tempdir");
    let database = directory.path().join("remote.db");
    let store = RemoteStore::open(&database).expect("store");
    service(store, &["ws_alpha"])
        .activate(vec![inventory("ws_alpha", Vec::new())])
        .expect("activate");

    let handles = (0..24)
        .map(|index| {
            let database = database.clone();
            std::thread::spawn(move || {
                let store = RemoteStore::open(&database).expect("thread store");
                service(store, &["ws_alpha"])
                    .allocate(
                        &request("ws_alpha", KnowledgeIdKind::Learning),
                        &context("ws_alpha", &format!("mcall-concurrent-{index}")),
                    )
                    .expect("concurrent allocation")
            })
        })
        .collect::<Vec<_>>();
    let mut allocations = handles
        .into_iter()
        .map(|handle| handle.join().expect("allocation thread"))
        .collect::<Vec<_>>();
    allocations.sort_by_key(|allocation| allocation.sequence);
    assert_eq!(
        allocations
            .iter()
            .map(|allocation| allocation.sequence)
            .collect::<Vec<_>>(),
        (1..=24).collect::<Vec<_>>()
    );
    assert_eq!(
        allocations
            .iter()
            .map(|allocation| allocation.id.clone())
            .collect::<HashSet<_>>()
            .len(),
        24
    );
    let reopened = RemoteStore::open(&database).expect("reopen");
    let audits: i64 = reopened
        .connection()
        .lock()
        .expect("connection")
        .query_row(
            "SELECT COUNT(*) FROM audit_events WHERE tool_name = ?1",
            [HUB_KNOWLEDGE_ALLOCATION_METHOD_V1],
            |row| row.get(0),
        )
        .expect("audit count");
    assert_eq!(audits, 24);
}

#[test]
fn restart_is_idempotent_and_late_workspace_reconciliation_is_required() {
    let directory = tempfile::tempdir().expect("tempdir");
    let database = directory.path().join("remote.db");
    let initial = inventory(
        "ws_alpha",
        vec![legacy(
            KnowledgeIdKind::Adr,
            "ADR-0008",
            &["adr-file:superseded"],
        )],
    );
    let store = RemoteStore::open(&database).expect("store");
    let initial_service = service(store, &["ws_alpha"]);
    initial_service
        .activate(vec![initial.clone()])
        .expect("activate");
    drop(initial_service);

    let reopened = RemoteStore::open(&database).expect("reopen");
    let restarted = service(reopened.clone(), &["ws_alpha"]);
    let state = restarted
        .activate(vec![initial])
        .expect("idempotent activation");
    assert_eq!(state.status, HubKnowledgeAllocatorStatus::Active);
    assert_eq!(state.adr_next_sequence, 9);

    let expanded = service(reopened.clone(), &["ws_alpha", "ws_beta"]);
    let error = expanded
        .allocate(
            &request("ws_beta", KnowledgeIdKind::Adr),
            &context("ws_beta", "mcall-too-early"),
        )
        .expect_err("late workspace is ineligible")
        .to_string();
    assert!(error.contains("not eligible"), "{error}");
    expanded
        .reconcile_workspace(inventory(
            "ws_beta",
            vec![legacy(
                KnowledgeIdKind::Adr,
                "ADR-0015",
                &["allocation:abandoned"],
            )],
        ))
        .expect("late reconciliation");
    let allocation = expanded
        .allocate(
            &request("ws_beta", KnowledgeIdKind::Adr),
            &context("ws_beta", "mcall-late"),
        )
        .expect("late workspace allocation");
    assert_eq!(allocation.id, "ADR-0016");
}

#[test]
fn late_reconciliation_reports_all_collisions_and_ledger_is_immutable() {
    let store = RemoteStore::open_in_memory().expect("store");
    let initial = service(store.clone(), &["ws_a"]);
    initial
        .activate(vec![inventory(
            "ws_a",
            vec![
                legacy(KnowledgeIdKind::Adr, "ADR-0002", &["adr-file:accepted"]),
                legacy(
                    KnowledgeIdKind::Learning,
                    "L-0005",
                    &["learning-file:active"],
                ),
            ],
        )])
        .expect("activate");
    let expanded = service(store.clone(), &["ws_a", "ws_b"]);
    let error = expanded
        .reconcile_workspace(inventory(
            "ws_b",
            vec![
                legacy(KnowledgeIdKind::Adr, "ADR-00002", &["allocation:merged"]),
                legacy(
                    KnowledgeIdKind::Learning,
                    "L-00005",
                    &["allocation:reserved"],
                ),
            ],
        ))
        .expect_err("collisions")
        .to_string();
    for expected in ["ADR-0002", "ADR-00002", "L-0005", "L-00005", "ws_a", "ws_b"] {
        assert!(error.contains(expected), "missing '{expected}' in {error}");
    }

    let allocation = initial
        .allocate(
            &request("ws_a", KnowledgeIdKind::Adr),
            &context("ws_a", "mcall-immutable"),
        )
        .expect("allocation");
    let connection = store.connection();
    let connection = connection.lock().expect("connection");
    let update = connection
        .execute(
            "UPDATE hub_knowledge_allocation_ledger SET id = 'ADR-9999' WHERE mcp_call_id = ?1",
            params![allocation.mcp_call_id],
        )
        .expect_err("ledger update must fail")
        .to_string();
    assert!(update.contains("immutable"), "{update}");
    let delete = connection
        .execute(
            "DELETE FROM hub_knowledge_allocation_ledger WHERE mcp_call_id = ?1",
            params![allocation.mcp_call_id],
        )
        .expect_err("ledger delete must fail")
        .to_string();
    assert!(delete.contains("immutable"), "{delete}");
}

#[test]
fn overflow_and_invalid_identity_do_not_advance_or_leave_partial_rows() {
    let store = RemoteStore::open_in_memory().expect("store");
    let service = service(store.clone(), &["ws_alpha"]);
    service
        .activate(vec![inventory("ws_alpha", Vec::new())])
        .expect("activate");
    store
        .connection()
        .lock()
        .expect("connection")
        .execute(
            "UPDATE hub_knowledge_sequences SET next_sequence = 4294967296 WHERE kind = 'adr'",
            [],
        )
        .expect("seed exhaustion");
    let error = service
        .allocate(
            &request("ws_alpha", KnowledgeIdKind::Adr),
            &context("ws_alpha", "mcall-overflow"),
        )
        .expect_err("overflow must fail")
        .to_string();
    assert!(error.contains("exhausted"), "{error}");

    let mut wrong_context = context("ws_other", "mcall-invalid");
    wrong_context.workspace_id = Some("ws_other".to_string());
    let error = service
        .allocate(
            &request("ws_alpha", KnowledgeIdKind::Learning),
            &wrong_context,
        )
        .expect_err("workspace identity mismatch")
        .to_string();
    assert!(error.contains("does not match"), "{error}");
    for (call_id, label) in [("", "empty"), ("  ", "whitespace")] {
        let mut invalid_correlation = context("ws_alpha", "placeholder");
        invalid_correlation.mcp_call_id = Some(call_id.to_string());
        let error = service
            .allocate(
                &request("ws_alpha", KnowledgeIdKind::Learning),
                &invalid_correlation,
            )
            .expect_err("invalid correlation must fail")
            .to_string();
        assert!(error.contains("mcp_call_id"), "{label}: {error}");
    }

    let connection = store.connection();
    let connection = connection.lock().expect("connection");
    let (adr_next, learning_next, ids, ledger, audits): (i64, i64, i64, i64, i64) = connection
        .query_row(
            "SELECT
                (SELECT next_sequence FROM hub_knowledge_sequences WHERE kind = 'adr'),
                (SELECT next_sequence FROM hub_knowledge_sequences WHERE kind = 'learning'),
                (SELECT COUNT(*) FROM hub_knowledge_ids),
                (SELECT COUNT(*) FROM hub_knowledge_allocation_ledger),
                (SELECT COUNT(*) FROM audit_events WHERE tool_name = ?1)",
            [HUB_KNOWLEDGE_ALLOCATION_METHOD_V1],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("unchanged state");
    assert_eq!(
        (adr_next, learning_next, ids, ledger, audits),
        (4294967296, 1, 0, 0, 0)
    );
}

#[test]
fn adr_and_learning_sequences_interleave_independently_and_allow_legacy_gaps() {
    let store = RemoteStore::open_in_memory().expect("store");
    let service = service(store, &["ws_alpha", "ws_beta"]);
    service
        .activate(vec![
            inventory("ws_alpha", Vec::new()),
            inventory("ws_beta", Vec::new()),
        ])
        .expect("activate");
    let adr_one = service
        .allocate(
            &request("ws_alpha", KnowledgeIdKind::Adr),
            &context("ws_alpha", "mcall-interleave-adr-1"),
        )
        .expect("adr one");
    let learning_one = service
        .allocate(
            &request("ws_alpha", KnowledgeIdKind::Learning),
            &context("ws_alpha", "mcall-interleave-learning-1"),
        )
        .expect("learning one");
    let adr_two = service
        .allocate(
            &request("ws_beta", KnowledgeIdKind::Adr),
            &context("ws_beta", "mcall-interleave-adr-2"),
        )
        .expect("adr two");
    assert_eq!(
        (
            adr_one.id.as_str(),
            learning_one.id.as_str(),
            adr_two.id.as_str()
        ),
        ("ADR-0001", "L-0001", "ADR-0002")
    );

    service
        .reconcile_workspace(inventory(
            "ws_beta",
            vec![legacy(
                KnowledgeIdKind::Adr,
                "ADR-0010",
                &["allocation:abandoned"],
            )],
        ))
        .expect("reconcile sparse legacy max");
    let adr_after_gap = service
        .allocate(
            &request("ws_alpha", KnowledgeIdKind::Adr),
            &context("ws_alpha", "mcall-interleave-adr-11"),
        )
        .expect("adr after gap");
    let learning_two = service
        .allocate(
            &request("ws_beta", KnowledgeIdKind::Learning),
            &context("ws_beta", "mcall-interleave-learning-2"),
        )
        .expect("learning two");
    assert_eq!(adr_after_gap.id, "ADR-0011");
    assert_eq!(learning_two.id, "L-0002");
}
