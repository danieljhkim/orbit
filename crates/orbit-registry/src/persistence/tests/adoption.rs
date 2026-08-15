use orbit_common::types::OrbitError;
use orbit_store::Store;

use super::super::{REGISTRY_SCHEMA_FEATURE, REGISTRY_SCHEMA_MIGRATIONS, RegistryStore};

#[test]
fn registry_store_adopts_registry_and_leaves_no_knowledge_allocation_schema() {
    let store = RegistryStore::open_in_memory().expect("registry store");
    let status = store.schema_status().expect("registry schema status");

    assert_eq!(status.feature, REGISTRY_SCHEMA_FEATURE);
    assert_eq!(status.current_version, 3);
    assert_eq!(status.applied.len(), 3);
    assert_eq!(status.applied[0].name, "adopt_global_v8_registry_schema");
    // [ORB-10725] The v2 slot keeps its shipped name — the ledger validates
    // names position by position — but no longer creates anything.
    assert_eq!(status.applied[1].name, "dormant_hub_knowledge_sequences");
    assert_eq!(
        status.applied[2].name,
        "drop_dormant_hub_knowledge_sequences"
    );
    assert!(status.pending.is_empty());

    for table in WITHDRAWN_KNOWLEDGE_TABLES {
        assert!(
            !remote_table_exists(&store, table),
            "a fresh database must never carry the withdrawn table '{table}'"
        );
    }
}

/// [ORB-10725] A database that applied the original dormant-substrate v2 opens
/// cleanly under the removal path: v3 drops what v2 built, and nothing about
/// the recorded ledger prefix has to change.
#[test]
fn registry_store_drops_the_withdrawn_knowledge_schema_from_a_v2_database() {
    let store = Store::open_in_memory().expect("store");
    seed_legacy_dormant_knowledge_schema(&store);
    for table in WITHDRAWN_KNOWLEDGE_TABLES {
        assert!(
            table_exists(&store, table),
            "seeded table '{table}' missing"
        );
    }

    let registry_store = RegistryStore::from_store(store.clone()).expect("open a v2 database");

    let status = registry_store
        .schema_status()
        .expect("registry schema status");
    assert_eq!(status.current_version, 3);
    assert_eq!(
        status
            .applied
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        [
            "adopt_global_v8_registry_schema",
            "dormant_hub_knowledge_sequences",
            "drop_dormant_hub_knowledge_sequences",
        ]
    );
    for table in WITHDRAWN_KNOWLEDGE_TABLES {
        assert!(
            !table_exists(&store, table),
            "the removal path left '{table}' behind"
        );
    }
    // Idempotent: reopening the same database applies nothing further.
    drop(RegistryStore::from_store(store.clone()).expect("reopen after removal"));
    for table in WITHDRAWN_KNOWLEDGE_TABLES {
        assert!(!table_exists(&store, table));
    }
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

    let _remote = RegistryStore::from_store(store.clone()).expect("adopt registry schema");

    assert_eq!(host_row_hex(&store), before);
}

#[test]
fn adoption_preserves_registry_schema_definitions_and_reopens_idempotently() {
    let store = Store::open_in_memory().expect("store");
    let before = registry_schema_definitions(&store);

    drop(RegistryStore::from_store(store.clone()).expect("first adoption"));
    let after_first = registry_schema_definitions(&store);
    drop(RegistryStore::from_store(store.clone()).expect("idempotent reopen"));

    assert_eq!(after_first, before);
    assert_eq!(registry_schema_definitions(&store), before);
}

#[test]
fn registry_store_refuses_a_future_legacy_feature_version() {
    let store = Store::open_in_memory().expect("store");
    drop(RegistryStore::from_store(store.clone()).expect("adopt current feature schema"));
    store
        .with_transaction(|tx| {
            tx.connection()
                .execute(
                    "INSERT INTO feature_schema_meta(feature, version, name, applied_at)
                     VALUES (?1, 4, 'future_remote_schema', '2026-07-19T00:00:00Z')",
                    [REGISTRY_SCHEMA_FEATURE],
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            Ok(())
        })
        .expect("seed future registry feature version");

    let error = match RegistryStore::from_store(store) {
        Ok(_) => panic!("older RegistryStore must reject a future feature version"),
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

    let error = match RegistryStore::from_store(store.clone()) {
        Ok(_) => panic!("adoption must reject a missing shipped trigger"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("host_aliases_immutable_delete"), "{error}");
    assert!(error.contains("missing"), "{error}");

    let status = store
        .feature_schema_status(REGISTRY_SCHEMA_FEATURE, REGISTRY_SCHEMA_MIGRATIONS)
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

    let error = match RegistryStore::from_store(store.clone()) {
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

    let error = match RegistryStore::from_store(store.clone()) {
        Ok(_) => panic!("adoption must reject changed table constraints"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("definition"), "{error}");
    assert!(error.contains("length(root)>0"), "{error}");
    assert_feature_v1_not_recorded(&store);
}

#[test]
fn configured_database_path_is_preserved_across_registry_store_reopen() {
    let directory = tempfile::tempdir().expect("tempdir");
    let configured = directory.path().join("configured-registry.db");
    let unrelated = directory.path().join("unrelated.db");

    let store = RegistryStore::open(&configured).expect("configured registry store");
    store
        .register_host(&orbit_common::types::HostRegistration {
            machine_id: "hm_configured".to_string(),
            host_id: "configured".to_string(),
            labels: Default::default(),
        })
        .expect("persist configured host");
    drop(store);

    assert!(configured.is_file());
    let reopened = RegistryStore::open(&configured).expect("reopen configured database");
    assert!(
        reopened
            .get_host("hm_configured")
            .expect("read host")
            .is_some()
    );

    let other = RegistryStore::open(&unrelated).expect("open unrelated database");
    assert!(
        other
            .get_host("hm_configured")
            .expect("read other")
            .is_none()
    );
}

fn assert_feature_v1_not_recorded(store: &Store) {
    let status = store
        .feature_schema_status(REGISTRY_SCHEMA_FEATURE, REGISTRY_SCHEMA_MIGRATIONS)
        .expect("failed adoption left feature ledger readable");
    assert_eq!(status.current_version, 0);
    assert!(status.applied.is_empty());
}

/// Every object the withdrawn [ORB-10272] substrate created, as named by the
/// original legacy feature-v2 migration.
const WITHDRAWN_KNOWLEDGE_TABLES: &[&str] = &[
    "hub_knowledge_allocator_state",
    "hub_knowledge_sequences",
    "hub_knowledge_ids",
    "hub_knowledge_workspace_reconciliation",
    "hub_knowledge_allocation_ledger",
];

/// Reproduce a database that applied the original legacy feature-v2 migration: its
/// tables on disk plus the ledger prefix recording v1 and v2 as applied. v1 is
/// validation-only over the shipped global schema, so recording it without
/// re-running it matches what a real v2 database looks like.
fn seed_legacy_dormant_knowledge_schema(store: &Store) {
    store
        .with_transaction(|tx| {
            let conn = tx.connection();
            conn.execute_batch(
                r#"
                CREATE TABLE hub_knowledge_allocator_state (
                    id                    INTEGER PRIMARY KEY CHECK (id = 0),
                    status                TEXT NOT NULL CHECK (status IN ('dormant', 'active')),
                    activation_generation INTEGER NOT NULL DEFAULT 0
                                              CHECK (activation_generation >= 0),
                    activated_at          TEXT,
                    updated_at            TEXT NOT NULL
                );

                INSERT INTO hub_knowledge_allocator_state(
                    id, status, activation_generation, activated_at, updated_at
                ) VALUES (0, 'dormant', 0, NULL, '2026-07-19T00:00:00Z');

                CREATE TABLE hub_knowledge_sequences (
                    kind          TEXT PRIMARY KEY CHECK (kind IN ('adr', 'learning')),
                    next_sequence INTEGER NOT NULL,
                    updated_at    TEXT NOT NULL
                );

                INSERT INTO hub_knowledge_sequences(kind, next_sequence, updated_at) VALUES
                    ('adr', 1, '2026-07-19T00:00:00Z'),
                    ('learning', 1, '2026-07-19T00:00:00Z');

                CREATE TABLE hub_knowledge_ids (
                    kind          TEXT NOT NULL CHECK (kind IN ('adr', 'learning')),
                    id            TEXT NOT NULL,
                    workspace_id  TEXT NOT NULL,
                    sequence      INTEGER NOT NULL,
                    origin        TEXT NOT NULL CHECK (origin IN ('legacy', 'allocated')),
                    evidence_json TEXT NOT NULL,
                    recorded_at   TEXT NOT NULL,
                    PRIMARY KEY(kind, id),
                    UNIQUE(kind, sequence)
                );

                CREATE INDEX hub_knowledge_ids_workspace
                    ON hub_knowledge_ids(workspace_id, kind, sequence);

                CREATE TABLE hub_knowledge_workspace_reconciliation (
                    workspace_id              TEXT PRIMARY KEY,
                    source_digest             TEXT NOT NULL,
                    source_count              INTEGER NOT NULL,
                    adr_max                   INTEGER NOT NULL,
                    learning_max              INTEGER NOT NULL,
                    reconciliation_generation INTEGER NOT NULL,
                    reconciled_at             TEXT NOT NULL
                );

                CREATE TABLE hub_knowledge_allocation_ledger (
                    mcp_call_id           TEXT PRIMARY KEY,
                    workspace_id          TEXT NOT NULL,
                    kind                  TEXT NOT NULL CHECK (kind IN ('adr', 'learning')),
                    id                    TEXT NOT NULL,
                    sequence              INTEGER NOT NULL,
                    request_identity_json TEXT NOT NULL,
                    allocated_at          TEXT NOT NULL,
                    FOREIGN KEY(kind, id) REFERENCES hub_knowledge_ids(kind, id)
                        ON UPDATE RESTRICT ON DELETE RESTRICT
                );

                CREATE INDEX hub_knowledge_allocation_lookup
                    ON hub_knowledge_allocation_ledger(workspace_id, kind, id);

                CREATE TRIGGER hub_knowledge_allocation_ledger_immutable_update
                BEFORE UPDATE ON hub_knowledge_allocation_ledger
                BEGIN
                    SELECT RAISE(ABORT, 'hub knowledge allocation ledger is immutable');
                END;

                CREATE TRIGGER hub_knowledge_allocation_ledger_immutable_delete
                BEFORE DELETE ON hub_knowledge_allocation_ledger
                BEGIN
                    SELECT RAISE(ABORT, 'hub knowledge allocation ledger is immutable');
                END;
                "#,
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
            // A live allocation row: the drop must not depend on the substrate
            // being empty, and the ledger's immutable-delete trigger must not
            // block it.
            conn.execute(
                "INSERT INTO hub_knowledge_ids(
                     kind, id, workspace_id, sequence, origin, evidence_json, recorded_at
                 ) VALUES ('learning', 'L-0007', 'ws-000000', 7, 'legacy', '[]', '2026-07-19T00:00:00Z')",
                [],
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
            conn.execute(
                "INSERT INTO hub_knowledge_allocation_ledger(
                     mcp_call_id, workspace_id, kind, id, sequence,
                     request_identity_json, allocated_at
                 ) VALUES ('call-1', 'ws-000000', 'learning', 'L-0007', 7, '{}', '2026-07-19T00:00:00Z')",
                [],
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
            for (version, name) in [
                (1, "adopt_global_v8_registry_schema"),
                (2, "dormant_hub_knowledge_sequences"),
            ] {
                conn.execute(
                    "INSERT INTO feature_schema_meta(feature, version, name, applied_at)
                     VALUES (?1, ?2, ?3, '2026-07-19T00:00:00Z')",
                    rusqlite::params![REGISTRY_SCHEMA_FEATURE, version, name],
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            }
            Ok(())
        })
        .expect("seed a legacy feature-v2 database");
}

fn table_exists(store: &Store, table: &str) -> bool {
    store
        .with_read_connection(|conn| {
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| OrbitError::Store(error.to_string()))
        })
        .expect("inspect sqlite_master")
        != 0
}

fn remote_table_exists(store: &RegistryStore, table: &str) -> bool {
    store
        .read(|conn| {
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| OrbitError::Store(error.to_string()))
        })
        .expect("inspect sqlite_master")
        != 0
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
