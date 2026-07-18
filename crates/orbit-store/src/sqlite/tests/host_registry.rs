use std::collections::BTreeSet;

use orbit_common::types::{HostNameResolution, HostRegistration, HostStatus};

use crate::Store;

fn registration(machine_id: &str, host_id: &str, labels: &[&str]) -> HostRegistration {
    HostRegistration {
        machine_id: machine_id.to_string(),
        host_id: host_id.to_string(),
        labels: labels.iter().map(|label| (*label).to_string()).collect(),
    }
}

fn error_text(result: Result<impl std::fmt::Debug, orbit_common::types::OrbitError>) -> String {
    result.expect_err("operation must fail").to_string()
}

#[test]
fn deterministic_two_host_fixture_covers_registry_lifecycle_and_name_safety() {
    let store = Store::open_in_memory().expect("store");
    let alpha = registration("hm_alpha", "alpha", &["codex", "os:linux"]);
    let beta = registration("hm_beta", "beta", &["claude", "os:macos"]);

    let first_alpha = store.register_host(&alpha).expect("register alpha");
    let repeated_alpha = store.register_host(&alpha).expect("repeat alpha");
    assert_eq!(repeated_alpha, first_alpha, "registration must be a no-op");
    assert_eq!(first_alpha.status, HostStatus::Active);
    assert_eq!(first_alpha.last_seen_at, Some(first_alpha.registered_at));

    let incompatible_name = error_text(store.register_host(&registration(
        "hm_alpha",
        "alpha-renamed-by-register",
        &["codex", "os:linux"],
    )));
    assert!(incompatible_name.contains("alpha"));
    assert!(incompatible_name.contains("cannot rename"));
    let incompatible_labels =
        error_text(store.register_host(&registration("hm_alpha", "alpha", &["different"])));
    assert!(incompatible_labels.contains("alpha"));
    assert!(incompatible_labels.contains("different label"));

    let active_collision =
        error_text(store.register_host(&registration("hm_beta", "alpha", &["claude", "os:macos"])));
    assert!(active_collision.contains("host_id 'alpha'"));
    assert!(active_collision.contains("active"));
    assert_eq!(store.get_host("hm_beta").expect("beta lookup"), None);

    store.register_host(&beta).expect("register beta");
    store
        .rename_host("hm_alpha", "alpha-2")
        .expect("first rename");
    match store.resolve_host_id("alpha").expect("resolve first alias") {
        HostNameResolution::Alias { host, alias } => {
            assert_eq!(host.host_id, "alpha-2");
            assert_eq!(alias.alias_host_id, "alpha");
            assert!(alias.warning.contains("permanent tombstone alias"));
        }
        other => panic!("expected alias resolution, got {other:?}"),
    }
    store
        .rename_host("hm_alpha", "alpha-3")
        .expect("second rename");
    let aliases = store.list_host_aliases("hm_alpha").expect("alias history");
    let alias_names: BTreeSet<&str> = aliases
        .iter()
        .map(|alias| alias.alias_host_id.as_str())
        .collect();
    assert_eq!(alias_names, BTreeSet::from(["alpha", "alpha-2"]));
    for alias in &aliases {
        match store
            .resolve_host_id(&alias.alias_host_id)
            .expect("resolve chained alias")
        {
            HostNameResolution::Alias { host, .. } => assert_eq!(host.host_id, "alpha-3"),
            other => panic!("expected active alias, got {other:?}"),
        }
    }

    for forbidden_alias in ["alpha", "alpha-2"] {
        let error =
            error_text(store.register_host(&registration("hm_gamma", forbidden_alias, &[])));
        assert!(error.contains(forbidden_alias));
        assert!(error.contains("permanent tombstone alias"));
    }

    let retired = store.retire_host("hm_alpha").expect("retire alpha");
    assert_eq!(retired.status, HostStatus::Retired);
    assert!(retired.retired_at.is_some());
    assert_eq!(store.retire_host("hm_alpha").expect("re-retire"), retired);
    assert_eq!(
        store
            .list_active_hosts()
            .expect("active hosts")
            .iter()
            .map(|host| host.host_id.as_str())
            .collect::<Vec<_>>(),
        vec!["beta"]
    );

    match store
        .resolve_host_id("alpha-3")
        .expect("resolve retired current name")
    {
        HostNameResolution::Retired { host, alias } => {
            assert_eq!(host.machine_id, "hm_alpha");
            assert!(alias.is_none());
        }
        other => panic!("expected retired current resolution, got {other:?}"),
    }
    match store
        .resolve_host_id("alpha")
        .expect("resolve retired alias")
    {
        HostNameResolution::Retired { host, alias } => {
            assert_eq!(host.machine_id, "hm_alpha");
            assert_eq!(alias.expect("alias metadata").alias_host_id, "alpha");
        }
        other => panic!("expected retired alias resolution, got {other:?}"),
    }

    let retired_reuse = error_text(store.register_host(&registration("hm_gamma", "alpha-3", &[])));
    assert!(retired_reuse.contains("host_id 'alpha-3'"));
    assert!(retired_reuse.contains("retired"));
    let reactivation = error_text(store.register_host(&alpha));
    assert!(reactivation.contains("host_id 'alpha-3'"));
    assert!(reactivation.contains("cannot reactivate"));
    let rename_retired = error_text(store.rename_host("hm_alpha", "alpha-4"));
    assert!(rename_retired.contains("host_id 'alpha-3'"));
    assert!(rename_retired.contains("retired"));

    assert_eq!(
        store.resolve_host_id("missing").expect("unknown"),
        HostNameResolution::Unknown {
            host_id: "missing".to_string()
        }
    );
}

#[test]
fn rename_failure_rolls_back_current_name_and_alias_insert() {
    let store = Store::open_in_memory().expect("store");
    store
        .register_host(&registration("hm_alpha", "alpha", &[]))
        .expect("register");
    {
        let connection = store.connection();
        let conn = connection.lock().expect("connection");
        conn.execute_batch(
            "CREATE TRIGGER fail_host_alias_insert
             BEFORE INSERT ON host_aliases
             BEGIN SELECT RAISE(ABORT, 'injected alias failure'); END;",
        )
        .expect("install failure trigger");
    }

    let error = error_text(store.rename_host("hm_alpha", "alpha-2"));
    assert!(error.contains("alpha"));
    assert!(error.contains("injected alias failure"));
    let host = store
        .get_host("hm_alpha")
        .expect("host lookup")
        .expect("host exists");
    assert_eq!(host.host_id, "alpha", "host update must roll back");
    assert!(
        store
            .list_host_aliases("hm_alpha")
            .expect("aliases")
            .is_empty(),
        "alias insert must roll back"
    );
}

#[test]
fn aliases_are_immutable_and_cross_table_uniqueness_is_enforced() {
    let store = Store::open_in_memory().expect("store");
    store
        .register_host(&registration("hm_alpha", "alpha", &[]))
        .expect("register alpha");
    store
        .rename_host("hm_alpha", "alpha-2")
        .expect("rename alpha");

    let connection = store.connection();
    let conn = connection.lock().expect("connection");
    let update = conn
        .execute(
            "UPDATE host_aliases SET host_id = 'reclaimed' WHERE host_id = 'alpha'",
            [],
        )
        .expect_err("aliases cannot update");
    assert!(update.to_string().contains("immutable"));
    let delete = conn
        .execute("DELETE FROM host_aliases WHERE host_id = 'alpha'", [])
        .expect_err("aliases cannot delete");
    assert!(delete.to_string().contains("permanent"));
    let collision = conn
        .execute(
            "INSERT INTO hosts(
                machine_id, host_id, labels_json, status, registered_at,
                updated_at, retired_at, last_seen_at
             ) VALUES (
                'hm_beta', 'alpha', '[]', 'active',
                '2026-07-18T06:00:00Z', '2026-07-18T06:00:00Z', NULL, NULL
             )",
            [],
        )
        .expect_err("alias name cannot become current");
    assert!(collision.to_string().contains("permanent alias"));

    // If an external actor removes the schema backstop and creates an
    // inconsistent legacy state, typed resolution must fail closed rather
    // than choose one machine.
    conn.execute_batch("DROP TRIGGER hosts_host_id_not_alias_insert")
        .expect("remove trigger for corruption fixture");
    conn.execute(
        "INSERT INTO hosts(
            machine_id, host_id, labels_json, status, registered_at,
            updated_at, retired_at, last_seen_at
         ) VALUES (
            'hm_beta', 'alpha', '[]', 'active',
            '2026-07-18T06:00:00Z', '2026-07-18T06:00:00Z', NULL, NULL
         )",
        [],
    )
    .expect("inject inconsistent name owner");
    drop(conn);
    match store.resolve_host_id("alpha").expect("resolve collision") {
        HostNameResolution::Collision {
            host_id,
            machine_ids,
        } => {
            assert_eq!(host_id, "alpha");
            assert_eq!(machine_ids, vec!["hm_alpha", "hm_beta"]);
        }
        other => panic!("expected typed collision, got {other:?}"),
    }
}

#[test]
fn reopening_store_preserves_registry_semantics_and_migration_is_a_noop() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("orbit.db");
    let store = Store::open(&path).expect("store");
    store
        .register_host(&registration("hm_alpha", "alpha", &["codex"]))
        .expect("register");
    store.rename_host("hm_alpha", "alpha-2").expect("rename");
    let first_ledger = store.applied_migrations().expect("first ledger");
    drop(store);

    let reopened = Store::open(&path).expect("reopen");
    assert_eq!(
        reopened.applied_migrations().expect("second ledger"),
        first_ledger
    );
    let host = reopened
        .get_host("hm_alpha")
        .expect("host")
        .expect("host exists");
    assert_eq!(host.host_id, "alpha-2");
    assert_eq!(
        reopened
            .list_host_aliases("hm_alpha")
            .expect("aliases")
            .len(),
        1
    );
    assert!(matches!(
        reopened.resolve_host_id("alpha").expect("resolve"),
        HostNameResolution::Alias { .. }
    ));
}

#[test]
fn identifiers_reject_paths_instead_of_persisting_them() {
    let store = Store::open_in_memory().expect("store");
    for registration in [
        registration("/home/operator/.ssh/id", "alpha", &[]),
        registration("hm_alpha", "/workspace/orbit", &[]),
        registration("hm_alpha", "alpha", &["checkout:/workspace/orbit"]),
    ] {
        let error = error_text(store.register_host(&registration));
        assert!(error.contains("not a path"));
    }
    assert!(store.list_active_hosts().expect("active hosts").is_empty());
}
