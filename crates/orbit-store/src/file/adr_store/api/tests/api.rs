// Migrated from file/adr_store/api.rs per ORB-00231
use std::fs;

use tempfile::tempdir;

use super::super::*;

fn create_params(title: &str, body: &str) -> AdrCreateParams {
    AdrCreateParams {
        title: title.to_string(),
        owner: "claude".to_string(),
        related_features: Vec::new(),
        related_tasks: Vec::new(),
        tags: Vec::new(),
        paths: Vec::new(),
        body: body.to_string(),
    }
}

#[test]
fn add_adr_then_get_adr_round_trips_content_and_layout() {
    let tempdir = tempdir().expect("tempdir");
    let store = AdrFileStore::new(tempdir.path().to_path_buf());

    let adr = store
        .add_adr(create_params("Initial decision", "## Context\nA body."))
        .expect("add adr");

    assert_eq!(adr.id, "ADR-0001");
    assert_eq!(adr.status, AdrStatus::Proposed);
    assert_eq!(adr.title, "Initial decision");

    let dir = tempdir.path().join("proposed").join("ADR-0001");
    assert!(dir.join("adr.yaml").is_file());
    assert!(dir.join("body.md").is_file());
    let allocation = store
        .id_allocator
        .adr_allocation(&adr.id)
        .expect("allocation")
        .expect("allocation exists");
    assert_eq!(
        allocation.worktree_root,
        std::fs::canonicalize(tempdir.path()).expect("canonical tempdir")
    );
    assert_eq!(
        allocation.body_path.as_deref(),
        Some(std::path::Path::new("proposed/ADR-0001/body.md"))
    );

    let loaded = store
        .get_adr("ADR-0001")
        .expect("get adr")
        .expect("adr exists");
    assert_eq!(loaded, adr);
}

#[test]
fn add_adr_twice_allocates_sequential_ids() {
    let tempdir = tempdir().expect("tempdir");
    let store = AdrFileStore::new(tempdir.path().to_path_buf());

    let first = store
        .add_adr(create_params("first", "body 1"))
        .expect("add 1");
    let second = store
        .add_adr(create_params("second", "body 2"))
        .expect("add 2");

    assert_eq!(first.id, "ADR-0001");
    assert_eq!(second.id, "ADR-0002");
}

#[test]
fn update_adr_status_proposed_to_accepted_moves_dir_and_sets_accepted_at() {
    let tempdir = tempdir().expect("tempdir");
    let store = AdrFileStore::new(tempdir.path().to_path_buf());
    let adr = store.add_adr(create_params("Decide", "Body")).expect("add");

    store
        .update_adr_status(&adr.id, AdrStatus::Accepted)
        .expect("accept");

    assert!(
        !tempdir.path().join("proposed").join(&adr.id).exists(),
        "proposed dir must be gone"
    );
    let accepted_dir = tempdir.path().join("accepted").join(&adr.id);
    assert!(accepted_dir.is_dir(), "accepted dir must exist");

    let loaded = store.get_adr(&adr.id).expect("get").expect("adr exists");
    assert_eq!(loaded.status, AdrStatus::Accepted);
    assert!(loaded.accepted_at.is_some(), "accepted_at must be set");
    assert!(
        loaded.last_updated >= adr.last_updated,
        "last_updated must advance"
    );
}

#[test]
fn update_adr_status_same_state_is_idempotent_no_op() {
    let tempdir = tempdir().expect("tempdir");
    let store = AdrFileStore::new(tempdir.path().to_path_buf());
    let adr = store.add_adr(create_params("Decide", "Body")).expect("add");

    store
        .update_adr_status(&adr.id, AdrStatus::Proposed)
        .expect("idempotent same-state");

    let loaded = store.get_adr(&adr.id).expect("get").expect("adr exists");
    assert_eq!(loaded.status, AdrStatus::Proposed);
    assert!(loaded.accepted_at.is_none());
}

#[test]
fn update_adr_status_rejects_accepted_to_proposed() {
    let tempdir = tempdir().expect("tempdir");
    let store = AdrFileStore::new(tempdir.path().to_path_buf());
    let adr = store.add_adr(create_params("Decide", "Body")).expect("add");
    store
        .update_adr_status(&adr.id, AdrStatus::Accepted)
        .expect("accept");

    let err = store
        .update_adr_status(&adr.id, AdrStatus::Proposed)
        .expect_err("accepted -> proposed is rejected");
    assert!(
        matches!(err, OrbitError::AdrInvalidTransition(_)),
        "expected AdrInvalidTransition, got {err:?}"
    );
}

#[test]
fn update_adr_document_updates_title_body_and_bumps_last_updated() {
    let tempdir = tempdir().expect("tempdir");
    let store = AdrFileStore::new(tempdir.path().to_path_buf());
    let adr = store
        .add_adr(create_params("Initial", "Initial body"))
        .expect("add");
    let initial_updated = adr.last_updated;

    // Sleep-free freshness check: re-read, compare.
    store
        .update_adr_document(
            &adr.id,
            &AdrDocumentUpdateParams {
                title: Some("Revised".to_string()),
                body: Some("Revised body".to_string()),
                ..Default::default()
            },
        )
        .expect("update");

    let loaded = store.get_adr(&adr.id).expect("get").expect("adr exists");
    assert_eq!(loaded.title, "Revised");
    let body = fs::read_to_string(
        tempdir
            .path()
            .join("proposed")
            .join(&adr.id)
            .join("body.md"),
    )
    .expect("read body");
    assert_eq!(body, "Revised body");
    assert!(loaded.last_updated >= initial_updated);
}

#[test]
fn delete_adr_on_proposed_removes_directory_and_returns_true() {
    let tempdir = tempdir().expect("tempdir");
    let store = AdrFileStore::new(tempdir.path().to_path_buf());
    let adr = store.add_adr(create_params("Doomed", "Bye")).expect("add");

    let removed = store.delete_adr(&adr.id).expect("delete");
    assert!(removed);
    assert!(
        !tempdir.path().join("proposed").join(&adr.id).exists(),
        "directory must be gone"
    );
    assert!(
        store.get_adr(&adr.id).expect("get").is_none(),
        "adr must no longer be found"
    );
}

#[test]
fn delete_adr_missing_returns_false() {
    let tempdir = tempdir().expect("tempdir");
    let store = AdrFileStore::new(tempdir.path().to_path_buf());

    let removed = store.delete_adr("ADR-9999").expect("delete missing");
    assert!(!removed);
}

#[test]
fn list_adrs_returns_all_adrs_across_state_dirs() {
    let tempdir = tempdir().expect("tempdir");
    let store = AdrFileStore::new(tempdir.path().to_path_buf());

    let a = store.add_adr(create_params("A", "ba")).expect("a");
    let b = store.add_adr(create_params("B", "bb")).expect("b");
    let c = store.add_adr(create_params("C", "bc")).expect("c");

    store
        .update_adr_status(&b.id, AdrStatus::Accepted)
        .expect("accept b");
    store
        .update_adr_status(&c.id, AdrStatus::Accepted)
        .expect("accept c");
    store
        .update_adr_status(&c.id, AdrStatus::Superseded)
        .expect("supersede c");

    let mut listed = store.list_adrs().expect("list");
    listed.sort_by(|x, y| x.id.cmp(&y.id));

    let ids: Vec<String> = listed.iter().map(|adr| adr.id.clone()).collect();
    assert_eq!(ids, vec![a.id.clone(), b.id.clone(), c.id.clone()]);

    let statuses: Vec<AdrStatus> = listed.iter().map(|adr| adr.status).collect();
    assert_eq!(
        statuses,
        vec![
            AdrStatus::Proposed,
            AdrStatus::Accepted,
            AdrStatus::Superseded
        ]
    );
}

// ----- Index-integration tests (Phase 3) -------------------------------

fn store_with_index() -> (tempfile::TempDir, AdrFileStore) {
    let dir = tempdir().expect("tempdir");
    let index = Store::open_in_memory().expect("open in-memory store");
    let store = AdrFileStore::new_with_index(dir.path().to_path_buf(), index);
    (dir, store)
}

fn count_index_rows(store: &AdrFileStore) -> i64 {
    let index = store.index.as_ref().expect("index attached");
    let conn = index.connection();
    let guard = conn.lock().expect("lock");
    guard
        .query_row("SELECT COUNT(*) FROM adrs", [], |row| row.get(0))
        .expect("query count")
}

#[test]
fn add_adr_with_index_populates_index_row() {
    let (_dir, store) = store_with_index();
    let adr = store
        .add_adr(create_params("Indexed", "body"))
        .expect("add");
    assert_eq!(count_index_rows(&store), 1);

    let listed = store
        .list_adrs_filtered(None, None, None, None, None, None, None, None)
        .expect("list filtered");
    let ids: Vec<String> = listed.iter().map(|a| a.id.clone()).collect();
    assert_eq!(ids, vec![adr.id]);
}

#[test]
fn update_adr_status_with_index_reflects_in_filter() {
    let (_dir, store) = store_with_index();
    let adr = store
        .add_adr(create_params("Promote", "body"))
        .expect("add");
    store
        .update_adr_status(&adr.id, AdrStatus::Accepted)
        .expect("accept");

    let accepted = store
        .list_adrs_filtered(
            Some(AdrStatus::Accepted),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("list accepted");
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].id, adr.id);

    let proposed = store
        .list_adrs_filtered(
            Some(AdrStatus::Proposed),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("list proposed");
    assert!(proposed.is_empty(), "no proposed ADRs after promotion");
}

#[test]
fn delete_adr_with_index_removes_row() {
    let (_dir, store) = store_with_index();
    let adr = store.add_adr(create_params("Doomed", "body")).expect("add");
    assert_eq!(count_index_rows(&store), 1);

    let removed = store.delete_adr(&adr.id).expect("delete");
    assert!(removed);
    assert_eq!(count_index_rows(&store), 0);

    let listed = store
        .list_adrs_filtered(None, None, None, None, None, None, None, None)
        .expect("list filtered");
    assert!(listed.is_empty());
}

#[test]
fn list_adrs_filtered_by_owner() {
    let (_dir, store) = store_with_index();
    let claude = store
        .add_adr(AdrCreateParams {
            title: "by claude".to_string(),
            owner: "claude".to_string(),
            related_features: Vec::new(),
            related_tasks: Vec::new(),
            tags: Vec::new(),
            paths: Vec::new(),
            body: "body".to_string(),
        })
        .expect("add claude");
    let _codex = store
        .add_adr(AdrCreateParams {
            title: "by codex".to_string(),
            owner: "codex".to_string(),
            related_features: Vec::new(),
            related_tasks: Vec::new(),
            tags: Vec::new(),
            paths: Vec::new(),
            body: "body".to_string(),
        })
        .expect("add codex");

    let filtered = store
        .list_adrs_filtered(None, Some("claude"), None, None, None, None, None, None)
        .expect("filter by owner");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, claude.id);
    assert_eq!(filtered[0].owner, "claude");
}

#[test]
fn list_adrs_filtered_by_legacy_id() {
    let (_dir, store) = store_with_index();
    let target = store
        .add_adr(create_params("Target", "body"))
        .expect("add target");
    let _other = store
        .add_adr(create_params("Other", "body"))
        .expect("add other");

    store
        .update_adr_document(
            &target.id,
            &AdrDocumentUpdateParams {
                legacy_ids: Some(vec!["activity-job/ADR-039".to_string()]),
                ..Default::default()
            },
        )
        .expect("set legacy id");

    let filtered = store
        .list_adrs_filtered(
            None,
            None,
            None,
            None,
            Some("activity-job/ADR-039"),
            None,
            None,
            None,
        )
        .expect("filter by legacy id");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, target.id);
}

#[test]
fn rebuild_index_after_index_clear_recovers() {
    let (_dir, store) = store_with_index();
    let a = store.add_adr(create_params("A", "ba")).expect("a");
    let b = store.add_adr(create_params("B", "bb")).expect("b");
    let c = store.add_adr(create_params("C", "bc")).expect("c");

    // Wipe the index out from under the store.
    {
        let index = store.index.as_ref().expect("index attached");
        let conn = index.connection();
        let guard = conn.lock().expect("lock");
        guard.execute("DELETE FROM adrs", []).expect("wipe index");
    }
    assert_eq!(count_index_rows(&store), 0);

    store.rebuild_index().expect("rebuild");
    assert_eq!(count_index_rows(&store), 3);

    let listed = store
        .list_adrs_filtered(None, None, None, None, None, None, None, None)
        .expect("list rebuilt");
    let mut ids: Vec<String> = listed.iter().map(|a| a.id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec![a.id, b.id, c.id]);
}

#[test]
fn list_adrs_filtered_without_index_falls_back_to_filesystem() {
    // AdrFileStore::new constructs without an index; the filter path must
    // still work via in-memory filtering.
    let tempdir = tempdir().expect("tempdir");
    let store = AdrFileStore::new(tempdir.path().to_path_buf());
    let a = store
        .add_adr(create_params("First", "body"))
        .expect("add a");
    let b = store
        .add_adr(create_params("Second", "body"))
        .expect("add b");
    store
        .update_adr_status(&b.id, AdrStatus::Accepted)
        .expect("accept b");

    let accepted = store
        .list_adrs_filtered(
            Some(AdrStatus::Accepted),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("fallback filter");
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].id, b.id);

    let all = store
        .list_adrs_filtered(None, None, None, None, None, None, None, None)
        .expect("fallback list");
    // ID-desc sort: b was allocated after a.
    let ids: Vec<String> = all.iter().map(|adr| adr.id.clone()).collect();
    assert_eq!(ids, vec![b.id, a.id]);
}
