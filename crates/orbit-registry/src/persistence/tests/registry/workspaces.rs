use super::*;

#[test]
fn two_host_multi_workspace_fixture_covers_ownership_presence_profile_cas_and_freshness() {
    let store = RegistryStore::open_in_memory().expect("store");
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
    let store = RegistryStore::open(&path).expect("store");
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

    let reopened = RegistryStore::open(&path).expect("reopen");
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
