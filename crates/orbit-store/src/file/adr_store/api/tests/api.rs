// Migrated from file/adr_store/api.rs per ORB-00231
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::{TempDir, tempdir};

use super::super::*;
use crate::IdAllocatorConfig;

struct TwoWorktreeFixture {
    _temp: TempDir,
    semantic_db: PathBuf,
    local_root: PathBuf,
    sibling_root: PathBuf,
    local: AdrFileStore,
    sibling: AdrFileStore,
}

impl TwoWorktreeFixture {
    fn new() -> Self {
        let temp = tempdir().expect("tempdir");
        let local_root = temp.path().join("local");
        let sibling_root = temp.path().join("sibling");
        init_registered_worktrees(&local_root, &sibling_root);
        let shared_root = local_root.join(".orbit");
        fs::create_dir_all(&shared_root).expect("shared root");
        let semantic_db = shared_root.join("state/semantic.db");
        let local = Self::store(&shared_root, &local_root);
        let sibling = Self::store(&shared_root, &sibling_root);
        Self {
            _temp: temp,
            semantic_db,
            local_root,
            sibling_root,
            local,
            sibling,
        }
    }

    fn store(shared_root: &Path, worktree_root: &Path) -> AdrFileStore {
        let adr_root = worktree_root.join(".orbit/adrs");
        let allocator = IdAllocator::open(IdAllocatorConfig::new(
            shared_root.join("state/semantic.db"),
            shared_root.join("state/.id_alloc.lock"),
            shared_root.to_path_buf(),
            worktree_root.to_path_buf(),
            adr_root.clone(),
            worktree_root.join(".orbit/learnings"),
        ))
        .expect("allocator");
        AdrFileStore::new_with_index_and_allocator(
            adr_root,
            Store::open_in_memory().expect("index"),
            allocator,
        )
    }

    fn add_sibling(&self, title: &str, body: &str, status: AdrStatus) -> Adr {
        let adr = self
            .sibling
            .add_adr(create_params(title, body))
            .expect("add sibling ADR");
        if status == AdrStatus::Accepted {
            self.sibling
                .update_adr_status(&adr.id, AdrStatus::Accepted)
                .expect("accept sibling ADR");
            self.sibling
                .get_adr(&adr.id)
                .expect("read accepted")
                .expect("accepted exists")
        } else {
            adr
        }
    }

    fn bundle_dir(root: &Path, adr: &Adr) -> PathBuf {
        root.join(".orbit/adrs")
            .join(adr.status.cli_name())
            .join(&adr.id)
    }

    fn copy_to_local(&self, adr: &Adr, body: &str) {
        let source = Self::bundle_dir(&self.sibling_root, adr);
        let target = Self::bundle_dir(&self.local_root, adr);
        fs::create_dir_all(&target).expect("local bundle dir");
        fs::copy(source.join("adr.yaml"), target.join("adr.yaml")).expect("copy ADR yaml");
        fs::write(target.join("body.md"), body).expect("write local body");
    }

    fn allocation_bytes(&self, id: &str) -> Vec<u8> {
        serde_json::to_vec(
            &self
                .local
                .id_allocator
                .adr_allocation(id)
                .expect("allocation read")
                .expect("allocation exists"),
        )
        .expect("serialize allocation")
    }

    fn bundle_bytes(root: &Path, adr: &Adr) -> (Vec<u8>, Vec<u8>) {
        let dir = Self::bundle_dir(root, adr);
        (
            fs::read(dir.join("adr.yaml")).expect("read yaml bytes"),
            fs::read(dir.join("body.md")).expect("read body bytes"),
        )
    }
}

fn init_registered_worktrees(local_root: &Path, sibling_root: &Path) {
    fs::create_dir_all(local_root).expect("local worktree root");
    git_ok(local_root, &["init", "-q"]);
    git_ok(
        local_root,
        &["config", "user.email", "test@example.invalid"],
    );
    git_ok(local_root, &["config", "user.name", "Orbit Test"]);
    fs::write(local_root.join("README.md"), "fixture\n").expect("fixture file");
    git_ok(local_root, &["add", "README.md"]);
    git_ok(local_root, &["commit", "-q", "-m", "fixture"]);
    git_ok(
        local_root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "sibling",
            sibling_root.to_str().expect("UTF-8 sibling path"),
        ],
    );
}

fn git_ok(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

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
fn two_worktrees_resolve_all_four_states_for_proposed_and_accepted_adrs() {
    let fixture = TwoWorktreeFixture::new();

    for status in [AdrStatus::Proposed, AdrStatus::Accepted] {
        let body = format!("exact sibling body for {}", status.cli_name());
        let adr = fixture.add_sibling("Federated", &body, status);
        let allocation_before = fixture.allocation_bytes(&adr.id);

        let AdrArtifactResolution::Federated(federated) = fixture
            .local
            .resolve_adr_artifact(&adr.id)
            .expect("federated resolution")
        else {
            panic!("expected federated ADR")
        };
        assert_eq!(federated.adr.status, status);
        assert_eq!(federated.body, body);
        assert_eq!(
            federated.artifact_origin.mode,
            ArtifactOriginMode::Federated
        );
        assert_eq!(fixture.allocation_bytes(&adr.id), allocation_before);

        let local_body = format!("exact local body for {}", status.cli_name());
        fixture.copy_to_local(&adr, &local_body);
        let AdrArtifactResolution::Local(local) = fixture
            .local
            .resolve_adr_artifact(&adr.id)
            .expect("local resolution")
        else {
            panic!("expected local ADR")
        };
        assert_eq!(local.adr.status, status);
        assert_eq!(local.body, local_body);
        assert_eq!(local.artifact_origin.mode, ArtifactOriginMode::Local);
        assert_eq!(fixture.allocation_bytes(&adr.id), allocation_before);

        fixture
            .local
            .update_adr_document(
                &adr.id,
                &AdrDocumentUpdateParams {
                    title: Some("Landed and editable".to_string()),
                    ..Default::default()
                },
            )
            .expect("landed local ADR is editable");
        if status == AdrStatus::Proposed {
            fixture
                .local
                .update_adr_status(&adr.id, AdrStatus::Accepted)
                .expect("landed local ADR is acceptable");
        }
        assert_eq!(fixture.allocation_bytes(&adr.id), allocation_before);

        let unavailable = fixture.add_sibling("Unavailable", "body", status);
        fs::remove_file(
            TwoWorktreeFixture::bundle_dir(&fixture.sibling_root, &unavailable).join("body.md"),
        )
        .expect("remove body");
        assert!(matches!(
            fixture
                .local
                .resolve_adr_artifact(&unavailable.id)
                .expect("unavailable resolution"),
            AdrArtifactResolution::RemoteArtifactUnavailable(_)
        ));

        let unknown = if status == AdrStatus::Proposed {
            "ADR-9001"
        } else {
            "ADR-9002"
        };
        assert_eq!(
            fixture
                .local
                .resolve_adr_artifact(unknown)
                .expect("not found resolution"),
            AdrArtifactResolution::NotFound
        );
    }
}

#[test]
fn remote_resolution_rejects_stale_missing_unreadable_and_removed_bundles() {
    let fixture = TwoWorktreeFixture::new();

    let stale = fixture.add_sibling("Stale path", "body", AdrStatus::Proposed);
    rusqlite::Connection::open(&fixture.semantic_db)
        .expect("open allocator db")
        .execute(
            "UPDATE id_allocations SET body_path = 'stale/ADR-0001/body.md' WHERE id = ?1",
            [&stale.id],
        )
        .expect("stale body path");
    assert_remote_unavailable(&fixture, &stale.id);

    let missing_yaml = fixture.add_sibling("Missing yaml", "body", AdrStatus::Accepted);
    fs::remove_file(
        TwoWorktreeFixture::bundle_dir(&fixture.sibling_root, &missing_yaml).join("adr.yaml"),
    )
    .expect("remove yaml");
    assert_remote_unavailable(&fixture, &missing_yaml.id);

    let missing_body = fixture.add_sibling("Missing body", "body", AdrStatus::Proposed);
    fs::remove_file(
        TwoWorktreeFixture::bundle_dir(&fixture.sibling_root, &missing_body).join("body.md"),
    )
    .expect("remove body");
    assert_remote_unavailable(&fixture, &missing_body.id);

    let unreadable = fixture.add_sibling("Unreadable", "body", AdrStatus::Accepted);
    let unreadable_path =
        TwoWorktreeFixture::bundle_dir(&fixture.sibling_root, &unreadable).join("body.md");
    fs::remove_file(&unreadable_path).expect("remove readable body");
    fs::create_dir(&unreadable_path).expect("replace body with unreadable directory");
    assert_remote_unavailable(&fixture, &unreadable.id);

    let removed = fixture.add_sibling("Removed worktree", "body", AdrStatus::Proposed);
    fs::remove_dir_all(&fixture.sibling_root).expect("remove sibling worktree");
    assert_remote_unavailable(&fixture, &removed.id);
}

#[test]
fn reconcile_breaks_the_readable_federated_restore_deadlock_without_reallocation() {
    let fixture = TwoWorktreeFixture::new();
    let old = fixture
        .sibling
        .add_adr(create_params(
            "Published history",
            "rejected alternative body",
        ))
        .expect("add old ADR");
    let replacement = fixture.add_sibling("Replacement", "replacement body", AdrStatus::Accepted);
    fixture
        .sibling
        .supersede_adr(&old.id, &replacement.id)
        .expect("supersede old ADR");
    let old = fixture
        .sibling
        .get_adr(&old.id)
        .expect("read superseded ADR")
        .expect("superseded ADR exists");
    let source_bytes = TwoWorktreeFixture::bundle_bytes(&fixture.sibling_root, &old);
    let allocation_before = fixture.allocation_bytes(&old.id);

    let restore_error = fixture
        .local
        .restore_allocated_adr(&old.id, create_params("Cannot restore", "new body"))
        .expect_err("restore refuses a readable federated artifact");
    assert!(format!("{restore_error}").contains("still readable"));

    let reconciled = fixture
        .local
        .reconcile_federated_adr(&old.id, &fixture.sibling_root)
        .expect("reconcile federated ADR");
    assert_eq!(reconciled, old);
    assert_eq!(
        TwoWorktreeFixture::bundle_bytes(&fixture.local_root, &reconciled),
        source_bytes,
        "reconciliation must preserve the complete bundle byte-for-byte"
    );
    assert_eq!(fixture.allocation_bytes(&old.id), allocation_before);

    fixture
        .local
        .reconcile_federated_adr(&old.id, &fixture.sibling_root)
        .expect("byte-equivalent destination is idempotent");
    assert_eq!(fixture.allocation_bytes(&old.id), allocation_before);
}

#[test]
fn reconcile_rejects_unregistered_incomplete_and_divergent_sources_without_mutation() {
    let fixture = TwoWorktreeFixture::new();
    let adr = fixture.add_sibling("Federated", "source body", AdrStatus::Accepted);
    let allocation_before = fixture.allocation_bytes(&adr.id);
    let target = TwoWorktreeFixture::bundle_dir(&fixture.local_root, &adr);

    let unregistered = fixture._temp.path().join("unregistered");
    fs::create_dir_all(&unregistered).expect("unregistered source");
    let error = fixture
        .local
        .reconcile_federated_adr(&adr.id, &unregistered)
        .expect_err("unregistered source must fail");
    assert!(format!("{error}").contains("not a registered Git worktree"));
    assert!(!target.exists());

    let source_body = TwoWorktreeFixture::bundle_dir(&fixture.sibling_root, &adr).join("body.md");
    let body = fs::read(&source_body).expect("source body snapshot");
    fs::remove_file(&source_body).expect("make source incomplete");
    fixture
        .local
        .reconcile_federated_adr(&adr.id, &fixture.sibling_root)
        .expect_err("incomplete source must fail");
    assert!(!target.exists());
    fs::write(&source_body, body).expect("restore source body");

    let accepted_source = TwoWorktreeFixture::bundle_dir(&fixture.sibling_root, &adr);
    let proposed_source = fixture
        .sibling_root
        .join(".orbit/adrs/proposed")
        .join(&adr.id);
    fs::create_dir_all(proposed_source.parent().expect("proposed parent"))
        .expect("proposed partition");
    fs::rename(&accepted_source, &proposed_source).expect("mispartition source bundle");
    let error = fixture
        .local
        .reconcile_federated_adr(&adr.id, &fixture.sibling_root)
        .expect_err("metadata and lifecycle partition mismatch must fail");
    assert!(format!("{error}").contains("metadata status"));
    assert!(!target.exists());
    fs::rename(&proposed_source, &accepted_source).expect("restore source partition");

    fixture.copy_to_local(&adr, "divergent local body");
    let local_before = TwoWorktreeFixture::bundle_bytes(&fixture.local_root, &adr);
    let error = fixture
        .local
        .reconcile_federated_adr(&adr.id, &fixture.sibling_root)
        .expect_err("divergent destination must fail");
    assert!(format!("{error}").contains("not byte-equivalent"));
    assert_eq!(
        TwoWorktreeFixture::bundle_bytes(&fixture.local_root, &adr),
        local_before
    );
    assert_eq!(fixture.allocation_bytes(&adr.id), allocation_before);
}

#[test]
fn reconciliation_allocation_guard_refuses_a_stale_snapshot_before_mutation() {
    let fixture = TwoWorktreeFixture::new();
    let adr = fixture.add_sibling("Federated", "source body", AdrStatus::Accepted);
    let expected = fixture
        .local
        .id_allocator
        .adr_allocation(&adr.id)
        .expect("read allocation")
        .expect("allocation exists");
    rusqlite::Connection::open(&fixture.semantic_db)
        .expect("open allocator db")
        .execute(
            "UPDATE id_allocations SET branch = 'concurrent-change' WHERE id = ?1",
            [&adr.id],
        )
        .expect("mutate allocation snapshot");

    let mut mutated = false;
    let error = fixture
        .local
        .id_allocator
        .with_unchanged_adr_allocation(&expected, || {
            mutated = true;
            Ok(())
        })
        .expect_err("stale allocation snapshot must fail");
    assert!(format!("{error}").contains("changed concurrently"));
    assert!(!mutated, "mutation closure must not run");
    assert!(!TwoWorktreeFixture::bundle_dir(&fixture.local_root, &adr).exists());
}

fn assert_remote_unavailable(fixture: &TwoWorktreeFixture, id: &str) {
    assert!(matches!(
        fixture
            .local
            .resolve_adr_artifact(id)
            .expect("resolve unavailable"),
        AdrArtifactResolution::RemoteArtifactUnavailable(_)
    ));
}

#[test]
fn sibling_only_mutations_fail_before_changing_bundles_or_allocations() {
    let fixture = TwoWorktreeFixture::new();
    let remote_old = fixture.add_sibling("Remote old", "old body", AdrStatus::Proposed);
    let remote_new = fixture.add_sibling("Remote new", "new body", AdrStatus::Accepted);
    let old_bundle = TwoWorktreeFixture::bundle_bytes(&fixture.sibling_root, &remote_old);
    let new_bundle = TwoWorktreeFixture::bundle_bytes(&fixture.sibling_root, &remote_new);
    let old_allocation = fixture.allocation_bytes(&remote_old.id);
    let new_allocation = fixture.allocation_bytes(&remote_new.id);

    for error in [
        fixture.local.update_adr_document(
            &remote_old.id,
            &AdrDocumentUpdateParams {
                title: Some("must not write".to_string()),
                ..Default::default()
            },
        ),
        fixture
            .local
            .update_adr_status(&remote_old.id, AdrStatus::Accepted),
        fixture.local.supersede_adr(&remote_old.id, &remote_new.id),
    ] {
        assert!(matches!(error, Err(OrbitError::ArtifactNotLocal { .. })));
    }

    let local_old = fixture
        .local
        .add_adr(create_params("Local old", "local old body"))
        .expect("local old");
    let local_new = fixture
        .local
        .add_adr(create_params("Local new", "local new body"))
        .expect("local new");
    fixture
        .local
        .update_adr_status(&local_new.id, AdrStatus::Accepted)
        .expect("accept local new");
    let local_old_before = TwoWorktreeFixture::bundle_bytes(&fixture.local_root, &local_old);
    let local_new_before = TwoWorktreeFixture::bundle_bytes(
        &fixture.local_root,
        &fixture
            .local
            .get_adr(&local_new.id)
            .expect("read local new")
            .expect("local new exists"),
    );

    assert!(matches!(
        fixture.local.supersede_adr(&local_old.id, &remote_new.id),
        Err(OrbitError::ArtifactNotLocal { .. })
    ));
    assert!(matches!(
        fixture.local.supersede_adr(&remote_old.id, &local_new.id),
        Err(OrbitError::ArtifactNotLocal { .. })
    ));

    assert_eq!(
        TwoWorktreeFixture::bundle_bytes(&fixture.sibling_root, &remote_old),
        old_bundle
    );
    assert_eq!(
        TwoWorktreeFixture::bundle_bytes(&fixture.sibling_root, &remote_new),
        new_bundle
    );
    assert_eq!(fixture.allocation_bytes(&remote_old.id), old_allocation);
    assert_eq!(fixture.allocation_bytes(&remote_new.id), new_allocation);
    assert_eq!(
        TwoWorktreeFixture::bundle_bytes(&fixture.local_root, &local_old),
        local_old_before
    );
    assert_eq!(
        TwoWorktreeFixture::bundle_bytes(
            &fixture.local_root,
            &fixture
                .local
                .get_adr(&local_new.id)
                .expect("read local new after")
                .expect("local new exists after"),
        ),
        local_new_before
    );
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
        .list_adrs_filtered(AdrListFilter::default())
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
        .list_adrs_filtered(AdrListFilter {
            status: Some(AdrStatus::Accepted),
            ..Default::default()
        })
        .expect("list accepted");
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].id, adr.id);

    let proposed = store
        .list_adrs_filtered(AdrListFilter {
            status: Some(AdrStatus::Proposed),
            ..Default::default()
        })
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
        .list_adrs_filtered(AdrListFilter::default())
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
        .list_adrs_filtered(AdrListFilter {
            owner: Some("claude"),
            ..Default::default()
        })
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
        .list_adrs_filtered(AdrListFilter {
            legacy_id: Some("activity-job/ADR-039"),
            ..Default::default()
        })
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
        .list_adrs_filtered(AdrListFilter::default())
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
        .list_adrs_filtered(AdrListFilter {
            status: Some(AdrStatus::Accepted),
            ..Default::default()
        })
        .expect("fallback filter");
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].id, b.id);

    let all = store
        .list_adrs_filtered(AdrListFilter::default())
        .expect("fallback list");
    // ID-desc sort: b was allocated after a.
    let ids: Vec<String> = all.iter().map(|adr| adr.id.clone()).collect();
    assert_eq!(ids, vec![b.id, a.id]);
}

// [ORB-10330] Preallocated owner-finalizer tests. The id is chosen upstream by
// the hub sequence, so the finalizer must never allocate, abandon, retry, or
// select a second id.

#[test]
fn finalize_preallocated_adr_writes_supplied_id_without_allocating() {
    let (_dir, store) = store_with_index();

    // A non-sequential id proves the id is not derived from a local sequence:
    // an allocation would have produced ADR-0001.
    let adr = store
        .finalize_preallocated_adr("ADR-0042", create_params("Preallocated", "hub body"))
        .expect("finalize preallocated ADR");
    assert_eq!(adr.id, "ADR-0042");
    assert_eq!(adr.status, AdrStatus::Proposed);

    // Body/bundle landed under the requested owner root.
    let bundle_dir = store.root.join("proposed/ADR-0042");
    assert!(bundle_dir.join("adr.yaml").is_file());
    assert_eq!(
        fs::read_to_string(bundle_dir.join("body.md")).expect("body"),
        "hub body"
    );

    // The projection records exactly the supplied id with its body path — and
    // nothing else. A stray ADR-0001 here would betray a hidden allocation.
    let allocations = store.id_allocator.adr_allocations().expect("allocations");
    assert_eq!(allocations.len(), 1, "no extra allocation was selected");
    assert_eq!(allocations[0].id, "ADR-0042");
    assert_eq!(
        allocations[0].body_path.as_deref(),
        Some(Path::new("proposed/ADR-0042/body.md"))
    );

    // Index projection exists so list works.
    assert_eq!(count_index_rows(&store), 1);
    let listed = store
        .list_adrs_filtered(AdrListFilter::default())
        .expect("list");
    assert_eq!(
        listed.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
        vec!["ADR-0042".to_string()]
    );
}

#[test]
fn finalize_preallocated_adr_rejects_existing_artifact_without_adopting() {
    let (_dir, store) = store_with_index();
    let original = store
        .finalize_preallocated_adr("ADR-0007", create_params("Original", "keep me"))
        .expect("seed original");
    let original_bytes =
        fs::read(store.root.join("proposed/ADR-0007/body.md")).expect("original body");

    let err = store
        .finalize_preallocated_adr("ADR-0007", create_params("Intruder", "overwrite"))
        .expect_err("collision must fail");
    assert!(matches!(err, OrbitError::InvalidInput(_)), "got {err:?}");

    // Original artifact and its allocation stay inspectable and unchanged.
    assert_eq!(
        fs::read(store.root.join("proposed/ADR-0007/body.md")).expect("still there"),
        original_bytes
    );
    let reread = store.get_adr("ADR-0007").expect("read").expect("exists");
    assert_eq!(reread.title, original.title);
    let allocations = store.id_allocator.adr_allocations().expect("allocations");
    assert_eq!(allocations.len(), 1);
    assert_eq!(allocations[0].id, "ADR-0007");
}

#[test]
fn finalize_preallocated_adr_cleans_up_partial_body_on_projection_failure() {
    let (_dir, store) = store_with_index();
    // Pre-seed a projection row for the target id so the finalizer's projection
    // insert conflicts *after* the body is written — the injected post-id
    // failure must leave no partial body behind.
    store
        .id_allocator
        .project_preallocated_adr("ADR-0099", Path::new("proposed/ADR-0099/body.md"))
        .expect("seed conflicting projection");

    let err = store
        .finalize_preallocated_adr("ADR-0099", create_params("Doomed", "body"))
        .expect_err("projection conflict must fail");
    assert!(matches!(err, OrbitError::Store(_)), "got {err:?}");

    // No partial bundle remains on disk.
    assert!(!store.root.join("proposed/ADR-0099").exists());
    // The pre-existing projection row is untouched (still exactly one row).
    let allocations = store.id_allocator.adr_allocations().expect("allocations");
    assert_eq!(allocations.len(), 1);
    assert_eq!(allocations[0].id, "ADR-0099");
}

#[test]
fn finalize_preallocated_adr_targets_only_the_selected_worktree() {
    let fixture = TwoWorktreeFixture::new();

    let adr = fixture
        .local
        .finalize_preallocated_adr("ADR-0100", create_params("Owner local", "local body"))
        .expect("finalize in local checkout");
    assert_eq!(adr.id, "ADR-0100");

    // Body materialized only under the local worktree.
    assert!(
        TwoWorktreeFixture::bundle_dir(&fixture.local_root, &adr)
            .join("body.md")
            .is_file()
    );
    // The sibling checkout's filesystem is untouched.
    assert!(!TwoWorktreeFixture::bundle_dir(&fixture.sibling_root, &adr).exists());
    assert!(fixture.semantic_db.is_file());
}

// [ORB-10538] Exact-id repair tests. Unlike hub preallocation finalization,
// restore requires a live allocation and may only replace an unreadable body.

#[test]
fn restore_allocated_adr_preserves_id_and_reindexes() {
    let (_dir, store) = store_with_index();
    let allocation = store.id_allocator.allocate_adr().expect("allocate ADR");

    let restored = store
        .restore_allocated_adr(&allocation.id, create_params("Restored", "restored body"))
        .expect("restore allocated ADR");

    assert_eq!(restored.id, allocation.id);
    assert_eq!(count_index_rows(&store), 1);
    let allocations = store.id_allocator.adr_allocations().expect("allocations");
    assert_eq!(allocations.len(), 1, "restore must not allocate another id");
    assert_eq!(allocations[0].id, restored.id);
    assert_eq!(
        allocations[0].body_path.as_deref(),
        Some(Path::new("proposed/ADR-0001/body.md"))
    );
    let AdrArtifactResolution::Local(artifact) = store
        .resolve_adr_artifact(&restored.id)
        .expect("resolve restored ADR")
    else {
        panic!("restored ADR must resolve locally")
    };
    assert_eq!(artifact.body, "restored body");
}

#[test]
fn restore_allocated_adr_refuses_missing_allocation_and_lifecycle_collision() {
    let (_dir, store) = store_with_index();
    let missing = store
        .restore_allocated_adr("ADR-0042", create_params("Missing", "body"))
        .expect_err("missing allocation must fail");
    assert!(
        matches!(missing, OrbitError::InvalidInput(_)),
        "got {missing:?}"
    );
    assert!(!store.root.join("proposed/ADR-0042").exists());

    let allocation = store.id_allocator.allocate_adr().expect("allocate ADR");
    fs::create_dir_all(store.root.join("accepted").join(&allocation.id))
        .expect("seed unreadable lifecycle collision");
    let collision = store
        .restore_allocated_adr(&allocation.id, create_params("Collision", "body"))
        .expect_err("lifecycle collision must fail");
    assert!(
        matches!(collision, OrbitError::InvalidInput(_)),
        "got {collision:?}"
    );
    assert!(!store.root.join("proposed").join(&allocation.id).exists());
}

#[test]
fn restore_allocated_adr_refuses_readable_artifact_and_retry_without_overwrite() {
    let (_dir, store) = store_with_index();
    let original = store
        .add_adr(create_params("Original", "keep me"))
        .expect("seed readable ADR");
    let original_body = fs::read(
        store
            .root
            .join("proposed")
            .join(&original.id)
            .join("body.md"),
    )
    .expect("read original body");

    for attempt in ["first", "retry"] {
        let error = store
            .restore_allocated_adr(&original.id, create_params(attempt, "overwrite"))
            .expect_err("readable artifact must never be restored over");
        assert!(
            matches!(error, OrbitError::InvalidInput(_)),
            "got {error:?}"
        );
    }
    assert_eq!(
        fs::read(
            store
                .root
                .join("proposed")
                .join(&original.id)
                .join("body.md")
        )
        .expect("reread original body"),
        original_body
    );
}

// [ORB-10479] An allocation whose worktree was reaped is marked `abandoned`
// by ORB-10501's repair, which hides it from `adr_allocation`. That is exactly
// the population restore exists to repair, so it must still be reachable —
// and a successful restore makes the row live again.
#[test]
fn restore_allocated_adr_revives_an_abandoned_allocation() {
    let (_dir, store) = store_with_index();
    let allocation = store.id_allocator.allocate_adr().expect("allocate ADR");
    store
        .id_allocator
        .abandon_adr(&allocation.id)
        .expect("abandon allocation");
    assert!(
        store
            .id_allocator
            .adr_allocation(&allocation.id)
            .expect("read allocation")
            .is_none(),
        "an abandoned allocation must stay hidden from ordinary reads"
    );

    let restored = store
        .restore_allocated_adr(&allocation.id, create_params("Revived", "revived body"))
        .expect("restore abandoned allocation");
    assert_eq!(
        restored.id, allocation.id,
        "restore must reuse the exact id"
    );

    let live = store
        .id_allocator
        .adr_allocation(&allocation.id)
        .expect("read allocation")
        .expect("restored allocation must be live again");
    assert_eq!(
        live.body_path.as_deref(),
        Some(Path::new("proposed/ADR-0001/body.md"))
    );
    assert_eq!(
        store
            .id_allocator
            .adr_allocations()
            .expect("allocations")
            .len(),
        1,
        "restore must not allocate another id"
    );
    let AdrArtifactResolution::Local(artifact) = store
        .resolve_adr_artifact(&restored.id)
        .expect("resolve revived ADR")
    else {
        panic!("revived ADR must resolve locally")
    };
    assert_eq!(artifact.body, "revived body");
}

#[test]
fn restore_allocated_adr_cleans_up_when_allocation_snapshot_changes() {
    let (_dir, store) = store_with_index();
    let allocation = store.id_allocator.allocate_adr().expect("allocate ADR");
    let snapshot = store
        .id_allocator
        .adr_allocation(&allocation.id)
        .expect("read allocation")
        .expect("allocation exists");
    store
        .id_allocator
        .record_adr_body_path(&allocation.id, Path::new("moved/ADR-0001/body.md"))
        .expect("simulate concurrent allocation change");

    let error = store
        .restore_allocated_adr_from_snapshot(
            &allocation.id,
            create_params("Stale snapshot", "body"),
            snapshot,
        )
        .expect_err("stale allocation snapshot must fail");
    assert!(
        matches!(error, OrbitError::InvalidInput(_)),
        "got {error:?}"
    );
    assert!(!store.root.join("proposed").join(&allocation.id).exists());
    assert_eq!(count_index_rows(&store), 0);
}
