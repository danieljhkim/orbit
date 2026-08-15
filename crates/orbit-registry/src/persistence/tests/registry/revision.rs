use super::*;

#[test]
fn hub_registration_is_singular_atomic_and_advances_once() {
    let store = RegistryStore::open_in_memory().expect("store");
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
    let store = RegistryStore::open_in_memory().expect("store");
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
    let store = RegistryStore::open_in_memory().expect("store");
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
    let store = RegistryStore::open_in_memory().expect("store");
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
    let store = RegistryStore::open_in_memory().expect("store");
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
    let store = RegistryStore::open_in_memory().expect("store");
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
    let store = RegistryStore::open_in_memory().expect("store");
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
