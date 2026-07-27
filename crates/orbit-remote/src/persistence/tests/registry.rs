use std::collections::BTreeSet;
use std::path::PathBuf;

use chrono::{Duration, TimeZone, Utc};
use orbit_common::types::{
    ExecutionProfileCrewV1, ExecutionProfileShipV1, ExecutionProfileV1, HostNameResolution,
    HostRegistration, HostStatus, ProjectionFreshness, WorkspacePresenceDeclaration,
};

use super::super::RemoteStore;

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

fn execution_profile(
    workspace_id: &str,
    owner_machine_id: &str,
    observed_at: chrono::DateTime<Utc>,
) -> ExecutionProfileV1 {
    let mut profile = ExecutionProfileV1 {
        schema_version: 1,
        workspace_id: workspace_id.to_string(),
        owner_machine_id: owner_machine_id.to_string(),
        observed_at,
        config_digest: String::new(),
        default_crew: "sol".to_string(),
        crews: vec![ExecutionProfileCrewV1 {
            name: "sol".to_string(),
            provider: "codex".to_string(),
            model: "gpt-test".to_string(),
            backend: "cli".to_string(),
            description: Some("Systems implementation".to_string()),
            tags: vec!["hard".to_string()],
        }],
        ship: ExecutionProfileShipV1 {
            mode: "pr".to_string(),
            base_branch: "agent-main".to_string(),
            ship_closure_digest: "a".repeat(64),
        },
    };
    profile.config_digest = profile.compute_config_digest().expect("config digest");
    profile
}

#[test]
fn deterministic_two_host_fixture_covers_registry_lifecycle_and_name_safety() {
    let store = RemoteStore::open_in_memory().expect("store");
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
    let store = RemoteStore::open_in_memory().expect("store");
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
fn rename_preflight_matches_transaction_validation_and_never_mutates() {
    let store = RemoteStore::open_in_memory().expect("store");
    store
        .register_host(&registration("hm_alpha", "alpha", &[]))
        .expect("register alpha");
    store
        .register_host(&registration("hm_beta", "beta", &[]))
        .expect("register beta");
    let revision = store.registry_revision().expect("revision");
    let before = store
        .get_host("hm_alpha")
        .expect("host lookup")
        .expect("alpha exists");

    for rejected in [
        " leading",
        "trailing ",
        "path/name",
        "path\\name",
        "line\nbreak",
        "beta",
    ] {
        let preflight = error_text(store.validate_host_rename("hm_alpha", rejected));
        let transactional = error_text(store.rename_host("hm_alpha", rejected));
        assert_eq!(
            preflight, transactional,
            "preflight and transaction diverged for {rejected:?}"
        );
        assert_eq!(
            store.get_host("hm_alpha").expect("host lookup"),
            Some(before.clone())
        );
        assert_eq!(
            store.registry_revision().expect("revision after rejection"),
            revision
        );
        assert!(
            store
                .list_host_aliases("hm_alpha")
                .expect("aliases")
                .is_empty()
        );
    }
}

#[test]
fn aliases_are_immutable_and_cross_table_uniqueness_is_enforced() {
    let store = RemoteStore::open_in_memory().expect("store");
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
    let store = RemoteStore::open(&path).expect("store");
    store
        .register_host(&registration("hm_alpha", "alpha", &["codex"]))
        .expect("register");
    store.rename_host("hm_alpha", "alpha-2").expect("rename");
    let first_ledger = store.schema_status().expect("first feature ledger");
    drop(store);

    let reopened = RemoteStore::open(&path).expect("reopen");
    assert_eq!(
        reopened.schema_status().expect("second feature ledger"),
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
    let store = RemoteStore::open_in_memory().expect("store");
    for machine_id in [
        "/home/operator/.ssh/id",
        "dk1",
        "user@dk1",
        "ssh:dk1",
        "hm_ssh:dk1",
    ] {
        let error = error_text(store.register_host(&registration(machine_id, "alpha", &[])));
        assert!(error.contains("machine_id"), "unexpected: {error}");
    }
    for registration in [
        registration("hm_alpha", "/workspace/orbit", &[]),
        registration("hm_alpha", "alpha", &["checkout:/workspace/orbit"]),
    ] {
        let error = error_text(store.register_host(&registration));
        assert!(error.contains("not a path"));
    }
    assert!(store.list_active_hosts().expect("active hosts").is_empty());
}

#[test]
fn two_host_multi_workspace_fixture_covers_ownership_presence_profile_cas_and_freshness() {
    let store = RemoteStore::open_in_memory().expect("store");
    store
        .register_host(&registration("hm_alpha", "alpha", &["codex"]))
        .expect("register alpha");
    store
        .register_host(&registration("hm_beta", "beta", &["claude"]))
        .expect("register beta");

    let alpha_owner = store
        .bind_workspace_owner("ws_alpha", "hm_alpha")
        .expect("bind alpha owner");
    assert_eq!(
        store
            .bind_workspace_owner("ws_alpha", "hm_alpha")
            .expect("repeat binding"),
        alpha_owner
    );
    store
        .bind_workspace_owner("ws_beta", "hm_beta")
        .expect("bind beta owner");
    let duplicate_owner = error_text(store.bind_workspace_owner("ws_alpha", "hm_beta"));
    assert!(duplicate_owner.contains("already owned"));

    let now = Utc
        .with_ymd_and_hms(2026, 7, 18, 9, 0, 0)
        .single()
        .expect("timestamp");
    let alpha_presence = vec![
        WorkspacePresenceDeclaration {
            workspace_id: "ws_alpha".to_string(),
            root: PathBuf::from("/alpha/ws-alpha"),
            last_verified: now,
        },
        WorkspacePresenceDeclaration {
            workspace_id: "ws_beta".to_string(),
            root: PathBuf::from("/alpha/ws-beta"),
            last_verified: now,
        },
    ];
    store
        .replace_host_workspace_presence("hm_alpha", &alpha_presence, now)
        .expect("publish alpha presence");
    store
        .replace_host_workspace_presence(
            "hm_beta",
            &[WorkspacePresenceDeclaration {
                workspace_id: "ws_alpha".to_string(),
                root: PathBuf::from("/beta/ws-alpha"),
                last_verified: now,
            }],
            now,
        )
        .expect("non-owner presence is valid");
    assert_eq!(
        store
            .get_host("hm_alpha")
            .expect("host")
            .expect("alpha")
            .last_seen_at,
        Some(now)
    );
    let private = store
        .list_host_workspace_presence_private("hm_alpha")
        .expect("private presence");
    assert_eq!(private.len(), 2);
    assert_eq!(private[0].root, PathBuf::from("/alpha/ws-alpha"));

    // An insert failure after DELETE rolls back the whole replacement.
    {
        let connection = store.connection();
        let conn = connection.lock().expect("connection");
        conn.execute_batch(
            "CREATE TRIGGER fail_beta_presence
             BEFORE INSERT ON host_workspace_presence
             WHEN NEW.machine_id = 'hm_alpha' AND NEW.workspace_id = 'ws_beta'
             BEGIN SELECT RAISE(ABORT, 'injected presence failure'); END;",
        )
        .expect("install trigger");
    }
    let replacement_error = error_text(store.replace_host_workspace_presence(
        "hm_alpha",
        &[WorkspacePresenceDeclaration {
            workspace_id: "ws_beta".to_string(),
            root: PathBuf::from("/alpha/new-beta"),
            last_verified: now + Duration::minutes(1),
        }],
        now + Duration::minutes(1),
    ));
    assert!(replacement_error.contains("injected presence failure"));
    assert_eq!(
        store
            .list_host_workspace_presence_private("hm_alpha")
            .expect("rolled-back presence"),
        private
    );
    {
        let connection = store.connection();
        connection
            .lock()
            .expect("connection")
            .execute_batch("DROP TRIGGER fail_beta_presence")
            .expect("drop trigger");
    }
    store
        .replace_host_workspace_presence(
            "hm_alpha",
            &[WorkspacePresenceDeclaration {
                workspace_id: "ws_beta".to_string(),
                root: PathBuf::from("/alpha/new-beta"),
                last_verified: now + Duration::minutes(1),
            }],
            now + Duration::minutes(1),
        )
        .expect("replace presence");
    assert_eq!(
        store
            .list_host_workspace_presence_private("hm_alpha")
            .expect("replaced presence")
            .iter()
            .map(|presence| presence.workspace_id.as_str())
            .collect::<Vec<_>>(),
        vec!["ws_beta"]
    );
    let missing_presence = store
        .sanitized_workspace_presence(
            "hm_alpha",
            "ws_alpha",
            now + Duration::minutes(1),
            Duration::minutes(5),
        )
        .expect("missing presence");
    assert_eq!(missing_presence.freshness, ProjectionFreshness::Missing);
    assert!(
        !serde_json::to_string(&missing_presence)
            .expect("serialize sanitized presence")
            .contains("/alpha")
    );
    let stale_presence = store
        .sanitized_workspace_presence(
            "hm_beta",
            "ws_alpha",
            now + Duration::minutes(6),
            Duration::minutes(5),
        )
        .expect("stale presence");
    assert_eq!(stale_presence.freshness, ProjectionFreshness::Stale);

    let missing_profile = store
        .sanitized_execution_profile("ws_alpha", now, Duration::minutes(5))
        .expect("missing profile");
    assert_eq!(missing_profile.freshness, ProjectionFreshness::Missing);
    assert_eq!(
        missing_profile.owner_machine_id.as_deref(),
        Some("hm_alpha")
    );

    let profile = execution_profile("ws_alpha", "hm_alpha", now);
    let non_owner = error_text(store.publish_execution_profile(
        "hm_beta",
        0,
        &profile,
        now,
        Duration::minutes(10),
        Duration::minutes(2),
    ));
    assert!(non_owner.contains("authenticated caller"));
    let stale = execution_profile("ws_alpha", "hm_alpha", now - Duration::minutes(11));
    assert!(
        error_text(store.publish_execution_profile(
            "hm_alpha",
            0,
            &stale,
            now,
            Duration::minutes(10),
            Duration::minutes(2),
        ))
        .contains("already stale")
    );
    let future = execution_profile("ws_alpha", "hm_alpha", now + Duration::minutes(3));
    assert!(
        error_text(store.publish_execution_profile(
            "hm_alpha",
            0,
            &future,
            now,
            Duration::minutes(10),
            Duration::minutes(2),
        ))
        .contains("future-dated")
    );

    let first = store
        .publish_execution_profile(
            "hm_alpha",
            0,
            &profile,
            now,
            Duration::minutes(10),
            Duration::minutes(2),
        )
        .expect("first profile");
    assert_eq!(first.generation, 1);
    let mut refreshed = profile.clone();
    refreshed.observed_at = now + Duration::minutes(1);
    let unchanged = store
        .publish_execution_profile(
            "hm_alpha",
            1,
            &refreshed,
            now + Duration::minutes(1),
            Duration::minutes(10),
            Duration::minutes(2),
        )
        .expect("unchanged refresh");
    assert_eq!(unchanged.generation, 1);
    assert_eq!(unchanged.received_at, now + Duration::minutes(1));

    let mut changed = refreshed.clone();
    changed.crews[0].model = "gpt-test-new".to_string();
    changed.config_digest = changed.compute_config_digest().expect("changed digest");
    let advanced = store
        .publish_execution_profile(
            "hm_alpha",
            1,
            &changed,
            now + Duration::minutes(2),
            Duration::minutes(10),
            Duration::minutes(2),
        )
        .expect("semantic change");
    assert_eq!(advanced.generation, 2);
    let stale_generation = error_text(store.publish_execution_profile(
        "hm_alpha",
        1,
        &changed,
        now + Duration::minutes(2),
        Duration::minutes(10),
        Duration::minutes(2),
    ));
    assert!(stale_generation.contains("stale execution profile generation"));
    let older_observation = error_text(store.publish_execution_profile(
        "hm_alpha",
        2,
        &profile,
        now + Duration::minutes(2),
        Duration::minutes(10),
        Duration::minutes(2),
    ));
    assert!(older_observation.contains("older than the stored"));

    let current = store
        .sanitized_execution_profile("ws_alpha", now + Duration::minutes(3), Duration::minutes(5))
        .expect("current profile");
    assert_eq!(current.freshness, ProjectionFreshness::Current);
    assert_eq!(current.generation, Some(2));
    let stale = store
        .sanitized_execution_profile("ws_alpha", now + Duration::minutes(8), Duration::minutes(5))
        .expect("stale profile");
    assert_eq!(stale.freshness, ProjectionFreshness::Stale);
}

#[test]
fn reopen_preserves_workspace_coordination_projections() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("orbit.db");
    let now = Utc
        .with_ymd_and_hms(2026, 7, 18, 10, 0, 0)
        .single()
        .expect("timestamp");
    let store = RemoteStore::open(&path).expect("store");
    store
        .register_host(&registration("hm_alpha", "alpha", &[]))
        .expect("host");
    store
        .bind_workspace_owner("ws_alpha", "hm_alpha")
        .expect("ownership");
    store
        .replace_host_workspace_presence(
            "hm_alpha",
            &[WorkspacePresenceDeclaration {
                workspace_id: "ws_alpha".to_string(),
                root: PathBuf::from("/alpha/ws-alpha"),
                last_verified: now,
            }],
            now,
        )
        .expect("presence");
    store
        .publish_execution_profile(
            "hm_alpha",
            0,
            &execution_profile("ws_alpha", "hm_alpha", now),
            now,
            Duration::minutes(10),
            Duration::minutes(2),
        )
        .expect("profile");
    let ledger = store.schema_status().expect("feature ledger");
    drop(store);

    let reopened = RemoteStore::open(&path).expect("reopen");
    assert_eq!(reopened.schema_status().expect("feature ledger"), ledger);
    assert_eq!(
        reopened
            .get_workspace_ownership("ws_alpha")
            .expect("ownership")
            .expect("bound")
            .owner_machine_id,
        "hm_alpha"
    );
    assert_eq!(
        reopened
            .list_host_workspace_presence_private("hm_alpha")
            .expect("presence")
            .len(),
        1
    );
    assert_eq!(
        reopened
            .get_execution_profile("ws_alpha")
            .expect("profile")
            .expect("published")
            .generation,
        1
    );
}

#[test]
fn hub_registration_is_singular_atomic_and_advances_once() {
    let store = RemoteStore::open_in_memory().expect("store");
    let hub = registration("hm_hub", "hub", &["codex"]);

    store.register_hub(&hub).expect("register hub");
    assert_eq!(
        store.hub_machine_id().expect("hub identity").as_deref(),
        Some("hm_hub")
    );
    assert_eq!(store.registry_revision().expect("hub revision"), 1);

    store
        .register_hub(&hub)
        .expect("idempotent hub registration");
    assert_eq!(store.registry_revision().expect("unchanged revision"), 1);

    let error = error_text(store.register_hub(&registration("hm_other", "other", &[])));
    assert!(error.contains("second hub"), "unexpected: {error}");
    assert!(
        store.get_host("hm_other").expect("lookup other").is_none(),
        "a rejected second hub must not leave a registered host behind"
    );
    assert_eq!(
        store.hub_machine_id().expect("hub identity").as_deref(),
        Some("hm_hub")
    );
    assert_eq!(store.registry_revision().expect("unchanged revision"), 1);
}

#[test]
fn hub_registration_rolls_back_host_and_identity_when_revision_is_exhausted() {
    let store = RemoteStore::open_in_memory().expect("store");
    {
        let connection = store.connection();
        let conn = connection.lock().expect("connection");
        conn.execute(
            "UPDATE hub_registry_metadata SET registry_revision = ?1 WHERE id = 0",
            [i64::MAX],
        )
        .expect("seed max revision");
    }

    let error = error_text(store.register_hub(&registration("hm_hub", "hub", &[])));
    assert!(error.contains("INTEGER range"), "unexpected: {error}");
    assert!(store.get_host("hm_hub").expect("host lookup").is_none());
    assert_eq!(store.hub_machine_id().expect("hub identity"), None);
    assert_eq!(
        store.registry_revision().expect("max revision"),
        i64::MAX as u64
    );
}

#[test]
fn configured_hub_retirement_is_rejected_inside_store_transaction() {
    let store = RemoteStore::open_in_memory().expect("store");
    store
        .register_hub(&registration("hm_hub", "hub", &[]))
        .expect("register hub");
    let revision = store.registry_revision().expect("revision before retire");

    let error = error_text(store.retire_host("hm_hub"));
    assert!(
        error.contains("cannot retire itself"),
        "unexpected: {error}"
    );
    let hub = store
        .get_host("hm_hub")
        .expect("lookup hub")
        .expect("hub record");
    assert_eq!(hub.status, HostStatus::Active);
    assert_eq!(hub.retired_at, None);
    assert_eq!(
        store.registry_revision().expect("revision after retire"),
        revision
    );
}

#[test]
fn registry_revision_overflow_rolls_back_snapshot_payload_mutation() {
    let store = RemoteStore::open_in_memory().expect("store");
    store
        .register_host(&registration("hm_alpha", "alpha", &[]))
        .expect("register");
    {
        let connection = store.connection();
        let conn = connection.lock().expect("connection");
        conn.execute(
            "UPDATE hub_registry_metadata SET registry_revision = ?1 WHERE id = 0",
            [i64::MAX],
        )
        .expect("seed max revision");
    }

    let error = error_text(store.rename_host("hm_alpha", "alpha2"));
    assert!(error.contains("INTEGER range"), "unexpected: {error}");
    assert_eq!(
        store.registry_revision().expect("max revision"),
        i64::MAX as u64
    );
    assert_eq!(
        store
            .get_host("hm_alpha")
            .expect("host")
            .expect("registered")
            .host_id,
        "alpha",
        "the host rename must roll back when its revision cannot advance"
    );
    assert!(
        store
            .list_host_aliases("hm_alpha")
            .expect("aliases")
            .is_empty(),
        "the tombstone insert must roll back with the rename"
    );
}

#[test]
fn exact_presence_and_profile_replays_are_revision_noops() {
    let store = RemoteStore::open_in_memory().expect("store");
    store
        .register_host(&registration("hm_alpha", "alpha", &[]))
        .expect("register");
    store
        .bind_workspace_owner("ws-1", "hm_alpha")
        .expect("bind owner");
    let now = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
    let declarations = vec![WorkspacePresenceDeclaration {
        workspace_id: "ws-1".to_string(),
        root: PathBuf::from("/srv/ws-1"),
        last_verified: now,
    }];

    let first_presence = store
        .replace_host_workspace_presence("hm_alpha", &declarations, now)
        .expect("first presence");
    let after_presence = store.registry_revision().expect("presence revision");
    let replayed_presence = store
        .replace_host_workspace_presence("hm_alpha", &declarations, now)
        .expect("exact presence replay");
    assert_eq!(replayed_presence, first_presence);
    assert_eq!(
        store.registry_revision().expect("replay revision"),
        after_presence
    );

    store
        .replace_host_workspace_presence("hm_alpha", &declarations, now + Duration::seconds(1))
        .expect("new receipt");
    assert_eq!(
        store.registry_revision().expect("fresh receipt revision"),
        after_presence + 1
    );

    let profile = execution_profile("ws-1", "hm_alpha", now);
    let first_profile = store
        .publish_execution_profile(
            "hm_alpha",
            0,
            &profile,
            now,
            Duration::minutes(10),
            Duration::minutes(2),
        )
        .expect("first profile");
    let after_profile = store.registry_revision().expect("profile revision");
    let replayed_profile = store
        .publish_execution_profile(
            "hm_alpha",
            first_profile.generation,
            &profile,
            now,
            Duration::minutes(10),
            Duration::minutes(2),
        )
        .expect("exact profile replay");
    assert_eq!(replayed_profile, first_profile);
    assert_eq!(
        store.registry_revision().expect("replay revision"),
        after_profile
    );

    let refreshed = store
        .publish_execution_profile(
            "hm_alpha",
            first_profile.generation,
            &profile,
            now + Duration::seconds(1),
            Duration::minutes(10),
            Duration::minutes(2),
        )
        .expect("new profile receipt");
    assert_eq!(refreshed.generation, first_profile.generation);
    assert_eq!(refreshed.received_at, now + Duration::seconds(1));
    assert_eq!(
        store.registry_revision().expect("fresh receipt revision"),
        after_profile + 1
    );
}

#[test]
fn registry_revision_advances_once_per_snapshot_visible_mutation_and_skips_no_ops() {
    let store = RemoteStore::open_in_memory().expect("store");
    assert_eq!(store.registry_revision().expect("initial revision"), 0);

    // Register advances; repeating the identical registration is a no-op.
    store
        .register_host(&registration("hm_alpha", "alpha", &["codex"]))
        .expect("register alpha");
    assert_eq!(store.registry_revision().expect("after register"), 1);
    store
        .register_host(&registration("hm_alpha", "alpha", &["codex"]))
        .expect("idempotent register");
    assert_eq!(store.registry_revision().expect("after idempotent"), 1);

    // Rename to the same name is a no-op; a real rename advances.
    store.rename_host("hm_alpha", "alpha").expect("noop rename");
    assert_eq!(store.registry_revision().expect("after noop rename"), 1);
    store.rename_host("hm_alpha", "alpha2").expect("rename");
    assert_eq!(store.registry_revision().expect("after rename"), 2);

    // Owner binding advances; repeating the same binding is a no-op.
    store
        .bind_workspace_owner("ws-1", "hm_alpha")
        .expect("bind owner");
    assert_eq!(store.registry_revision().expect("after bind"), 3);
    store
        .bind_workspace_owner("ws-1", "hm_alpha")
        .expect("idempotent bind");
    assert_eq!(store.registry_revision().expect("after idempotent bind"), 3);

    // Presence and profile publication both advance (freshness-visible receipt).
    let now = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
    store
        .replace_host_workspace_presence(
            "hm_alpha",
            &[WorkspacePresenceDeclaration {
                workspace_id: "ws-1".to_string(),
                root: PathBuf::from("/srv/checkouts/ws-1"),
                last_verified: now,
            }],
            now,
        )
        .expect("presence");
    assert_eq!(store.registry_revision().expect("after presence"), 4);

    store
        .publish_execution_profile(
            "hm_alpha",
            0,
            &execution_profile("ws-1", "hm_alpha", now),
            now,
            Duration::minutes(10),
            Duration::minutes(2),
        )
        .expect("profile");
    assert_eq!(store.registry_revision().expect("after profile"), 5);

    // Retirement of a distinct host advances; repeating it is a no-op.
    store
        .register_host(&registration("hm_beta", "beta", &["claude"]))
        .expect("register beta");
    assert_eq!(store.registry_revision().expect("after beta"), 6);
    store.retire_host("hm_beta").expect("retire beta");
    assert_eq!(store.registry_revision().expect("after retire"), 7);
    store.retire_host("hm_beta").expect("idempotent retire");
    assert_eq!(
        store.registry_revision().expect("after idempotent retire"),
        7
    );
}

#[test]
fn registry_snapshot_is_path_free_and_carries_no_crew_or_model_content() {
    let store = RemoteStore::open_in_memory().expect("store");
    store
        .register_hub(&registration("hm_alpha", "alpha", &["codex"]))
        .expect("register hub");
    store.rename_host("hm_alpha", "alpha2").expect("rename");
    store
        .bind_workspace_owner("ws-1", "hm_alpha")
        .expect("bind owner");

    let now = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
    let secret_root = PathBuf::from("/srv/secret-checkouts/ws-1");
    store
        .replace_host_workspace_presence(
            "hm_alpha",
            &[WorkspacePresenceDeclaration {
                workspace_id: "ws-1".to_string(),
                root: secret_root.clone(),
                last_verified: now,
            }],
            now,
        )
        .expect("presence");
    store
        .publish_execution_profile(
            "hm_alpha",
            0,
            &execution_profile("ws-1", "hm_alpha", now),
            now,
            Duration::minutes(10),
            Duration::minutes(2),
        )
        .expect("profile");

    let snapshot = store
        .read_registry_snapshot(now, Duration::minutes(5), Duration::minutes(10))
        .expect("snapshot");

    assert_eq!(snapshot.hub_machine_id.as_deref(), Some("hm_alpha"));
    assert!(snapshot.registry_revision > 0);
    assert_eq!(snapshot.hosts.len(), 1);
    let host = &snapshot.hosts[0];
    assert_eq!(host.host_id, "alpha2");
    assert_eq!(host.aliases.len(), 1, "tombstone alias retained");
    assert_eq!(host.aliases[0].alias_host_id, "alpha");
    assert_eq!(host.presence.len(), 1);
    assert_eq!(host.presence[0].workspace_id, "ws-1");
    assert_eq!(host.presence[0].freshness, ProjectionFreshness::Current);

    assert_eq!(snapshot.workspaces.len(), 1);
    let workspace = &snapshot.workspaces[0];
    assert_eq!(workspace.owner_machine_id, "hm_alpha");
    assert_eq!(workspace.owner_host_id.as_deref(), Some("alpha2"));
    assert_eq!(workspace.profile.freshness, ProjectionFreshness::Current);
    assert_eq!(workspace.profile.generation, Some(1));

    // Hostile audit: the serialized snapshot must not carry any presence root,
    // crew name, model, or provider token.
    let json = serde_json::to_string(&snapshot).expect("serialize snapshot");
    for forbidden in [
        "/srv/secret-checkouts",
        "secret-checkouts",
        "gpt-test",
        "\"provider\"",
        "\"model\"",
        "config_digest",
        "ship_closure_digest",
    ] {
        assert!(
            !json.contains(forbidden),
            "snapshot leaked forbidden content '{forbidden}': {json}"
        );
    }
}
