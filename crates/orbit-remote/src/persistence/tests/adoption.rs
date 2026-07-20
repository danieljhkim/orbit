use orbit_common::types::OrbitError;
use orbit_store::Store;

use super::super::{REMOTE_SCHEMA_FEATURE, REMOTE_SCHEMA_MIGRATIONS, RemoteStore};

#[test]
fn remote_store_adopts_registry_and_installs_knowledge_cutover_schema_as_feature_v3() {
    let store = RemoteStore::open_in_memory().expect("remote store");
    let status = store.schema_status().expect("remote schema status");

    assert_eq!(status.feature, REMOTE_SCHEMA_FEATURE);
    assert_eq!(status.current_version, 3);
    assert_eq!(status.applied.len(), 3);
    assert_eq!(status.applied[0].name, "adopt_global_v8_registry_schema");
    assert_eq!(status.applied[1].name, "dormant_hub_knowledge_sequences");
    assert_eq!(status.applied[2].name, "knowledge_authority_cutover_state");
    assert!(status.pending.is_empty());

    let allocator = store
        .knowledge_allocator_state()
        .expect("dormant allocator state");
    assert_eq!(
        allocator.status,
        super::super::HubKnowledgeAllocatorStatus::Dormant
    );
    assert_eq!(allocator.activation_generation, 0);
    assert_eq!(allocator.adr_next_sequence, 1);
    assert_eq!(allocator.learning_next_sequence, 1);
}

#[test]
fn adoption_does_not_rewrite_existing_registry_row_bytes() {
    let store = Store::open_in_memory().expect("store");
    store
        .with_transaction(|tx| {
            tx.connection()
                .execute(
                    "INSERT INTO hosts(
                         machine_id, host_id, labels_json, status, registered_at,
                         updated_at, retired_at, last_seen_at
                     ) VALUES (
                         'hm_bytes', 'byte-host', '[\"opus\", \"codex\"]', 'active',
                         '2026-07-18T12:34:56.123456789+00:00',
                         '2026-07-18T12:35:00.000000001+00:00', NULL,
                         '2026-07-18T12:35:01.999999999+00:00'
                     )",
                    [],
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            Ok(())
        })
        .expect("seed shipped registry row");
    let before = host_row_hex(&store);

    let _remote = RemoteStore::from_store(store.clone()).expect("adopt registry schema");

    assert_eq!(host_row_hex(&store), before);
}

#[test]
fn adoption_preserves_registry_schema_definitions_and_reopens_idempotently() {
    let store = Store::open_in_memory().expect("store");
    let before = registry_schema_definitions(&store);

    drop(RemoteStore::from_store(store.clone()).expect("first adoption"));
    let after_first = registry_schema_definitions(&store);
    drop(RemoteStore::from_store(store.clone()).expect("idempotent reopen"));

    assert_eq!(after_first, before);
    assert_eq!(registry_schema_definitions(&store), before);
}

#[test]
fn remote_store_refuses_a_future_remote_feature_version() {
    let store = Store::open_in_memory().expect("store");
    drop(RemoteStore::from_store(store.clone()).expect("adopt current feature schema"));
    store
        .with_transaction(|tx| {
            tx.connection()
                .execute(
                    "INSERT INTO feature_schema_meta(feature, version, name, applied_at)
                     VALUES (?1, 4, 'future_remote_schema', '2026-07-19T00:00:00Z')",
                    [REMOTE_SCHEMA_FEATURE],
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            Ok(())
        })
        .expect("seed future Remote feature version");

    let error = match RemoteStore::from_store(store) {
        Ok(_) => panic!("older Remote binary must reject a future feature version"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("newer"), "{error}");
    assert!(error.contains("upgrade orbit"), "{error}");
}

#[test]
fn adoption_fails_closed_without_recording_v1_when_shipped_schema_is_damaged() {
    let store = Store::open_in_memory().expect("store");
    store
        .with_transaction(|tx| {
            tx.connection()
                .execute_batch("DROP TRIGGER host_aliases_immutable_delete")
                .map_err(|error| OrbitError::Store(error.to_string()))
        })
        .expect("damage shipped schema fixture");

    let error = match RemoteStore::from_store(store.clone()) {
        Ok(_) => panic!("adoption must reject a missing shipped trigger"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("host_aliases_immutable_delete"), "{error}");
    assert!(error.contains("missing"), "{error}");

    let status = store
        .feature_schema_status(REMOTE_SCHEMA_FEATURE, REMOTE_SCHEMA_MIGRATIONS)
        .expect("failed adoption left feature ledger readable");
    assert_eq!(status.current_version, 0);
    assert!(status.applied.is_empty());
}

#[test]
fn adoption_rejects_changed_foreign_key_delete_behavior() {
    let store = Store::open_in_memory().expect("store");
    store
        .with_transaction(|tx| {
            tx.connection()
                .execute_batch(
                    "DROP TABLE host_workspace_presence;
                     CREATE TABLE host_workspace_presence (
                         machine_id    TEXT NOT NULL,
                         workspace_id  TEXT NOT NULL,
                         root          TEXT NOT NULL,
                         last_verified TEXT NOT NULL,
                         PRIMARY KEY(machine_id, workspace_id),
                         CHECK (length(workspace_id) > 0),
                         CHECK (length(root) > 0),
                         FOREIGN KEY(machine_id) REFERENCES hosts(machine_id)
                             ON UPDATE RESTRICT ON DELETE CASCADE
                     );
                     CREATE INDEX idx_host_workspace_presence_workspace
                         ON host_workspace_presence(workspace_id, machine_id);",
                )
                .map_err(|error| OrbitError::Store(error.to_string()))
        })
        .expect("replace presence table with wrong delete behavior");

    let error = match RemoteStore::from_store(store.clone()) {
        Ok(_) => panic!("adoption must reject changed foreign-key behavior"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("foreign-key contract differs"), "{error}");
    assert!(error.contains("CASCADE"), "{error}");
    assert_feature_v1_not_recorded(&store);
}

#[test]
fn adoption_rejects_missing_table_constraint_with_matching_columns() {
    let store = Store::open_in_memory().expect("store");
    store
        .with_transaction(|tx| {
            tx.connection()
                .execute_batch(
                    "DROP TABLE host_workspace_presence;
                     CREATE TABLE host_workspace_presence (
                         machine_id    TEXT NOT NULL,
                         workspace_id  TEXT NOT NULL,
                         root          TEXT NOT NULL,
                         last_verified TEXT NOT NULL,
                         PRIMARY KEY(machine_id, workspace_id),
                         CHECK (length(workspace_id) > 0),
                         FOREIGN KEY(machine_id) REFERENCES hosts(machine_id)
                             ON UPDATE RESTRICT ON DELETE RESTRICT
                     );
                     CREATE INDEX idx_host_workspace_presence_workspace
                         ON host_workspace_presence(workspace_id, machine_id);",
                )
                .map_err(|error| OrbitError::Store(error.to_string()))
        })
        .expect("replace presence table without root constraint");

    let error = match RemoteStore::from_store(store.clone()) {
        Ok(_) => panic!("adoption must reject changed table constraints"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("definition"), "{error}");
    assert!(error.contains("length(root)>0"), "{error}");
    assert_feature_v1_not_recorded(&store);
}

#[test]
fn configured_database_path_is_preserved_across_remote_store_reopen() {
    let directory = tempfile::tempdir().expect("tempdir");
    let configured = directory.path().join("configured-remote.db");
    let unrelated = directory.path().join("unrelated.db");

    let store = RemoteStore::open(&configured).expect("configured remote store");
    store
        .register_host(&orbit_common::types::HostRegistration {
            machine_id: "hm_configured".to_string(),
            host_id: "configured".to_string(),
            labels: Default::default(),
        })
        .expect("persist configured host");
    drop(store);

    assert!(configured.is_file());
    let reopened = RemoteStore::open(&configured).expect("reopen configured database");
    assert!(
        reopened
            .get_host("hm_configured")
            .expect("read host")
            .is_some()
    );

    let other = RemoteStore::open(&unrelated).expect("open unrelated database");
    assert!(
        other
            .get_host("hm_configured")
            .expect("read other")
            .is_none()
    );
}

fn assert_feature_v1_not_recorded(store: &Store) {
    let status = store
        .feature_schema_status(REMOTE_SCHEMA_FEATURE, REMOTE_SCHEMA_MIGRATIONS)
        .expect("failed adoption left feature ledger readable");
    assert_eq!(status.current_version, 0);
    assert!(status.applied.is_empty());
}

fn host_row_hex(store: &Store) -> String {
    store
        .with_read_connection(|conn| {
            conn.query_row(
                "SELECT
                     hex(CAST(machine_id AS BLOB)) || '|' ||
                     hex(CAST(host_id AS BLOB)) || '|' ||
                     hex(CAST(labels_json AS BLOB)) || '|' ||
                     hex(CAST(status AS BLOB)) || '|' ||
                     hex(CAST(registered_at AS BLOB)) || '|' ||
                     hex(CAST(updated_at AS BLOB)) || '|' ||
                     COALESCE(hex(CAST(retired_at AS BLOB)), '<null>') || '|' ||
                     hex(CAST(last_seen_at AS BLOB))
                 FROM hosts WHERE machine_id = 'hm_bytes'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| OrbitError::Store(error.to_string()))
        })
        .expect("read registry row bytes")
}

fn registry_schema_definitions(store: &Store) -> Vec<(String, String, String)> {
    store
        .with_read_connection(|conn| {
            let mut statement = conn
                .prepare(
                    "SELECT type, name, sql
                     FROM sqlite_master
                     WHERE name IN (
                         'hosts',
                         'host_aliases',
                         'workspace_ownership',
                         'host_workspace_presence',
                         'workspace_execution_profiles',
                         'hub_registry_metadata',
                         'idx_hosts_status_host_id',
                         'idx_host_aliases_machine_id',
                         'idx_workspace_ownership_owner',
                         'idx_host_workspace_presence_workspace',
                         'hosts_host_id_not_alias_insert',
                         'hosts_host_id_not_alias_update',
                         'host_alias_not_current_name_insert',
                         'host_aliases_immutable_update',
                         'host_aliases_immutable_delete',
                         'execution_profile_owner_matches_insert',
                         'execution_profile_owner_matches_update'
                     )
                     ORDER BY type, name",
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| OrbitError::Store(error.to_string()))
        })
        .expect("read registry schema definitions")
}
