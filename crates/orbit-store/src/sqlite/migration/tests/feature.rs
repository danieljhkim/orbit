use orbit_common::OrbitError;
use rusqlite::{Connection, params};

use crate::Store;

use super::super::{FeatureMigration, table_exists};

const FEATURE: &str = "orbit-test-feature";

fn migration_v1(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch("CREATE TABLE feature_test_v1 (value TEXT NOT NULL)")
        .map_err(|error| OrbitError::Store(error.to_string()))
}

fn migration_v2(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch("CREATE TABLE feature_test_v2 (value TEXT NOT NULL)")
        .map_err(|error| OrbitError::Store(error.to_string()))
}

fn migration_v2_fails(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch("CREATE TABLE feature_test_half_applied (value TEXT NOT NULL)")
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    Err(OrbitError::Migration(
        "intentional feature migration failure".to_string(),
    ))
}

fn registry() -> [FeatureMigration; 2] {
    [
        FeatureMigration::new(1, "create_v1", migration_v1),
        FeatureMigration::new(2, "create_v2", migration_v2),
    ]
}

#[test]
fn feature_schema_applies_in_order_reports_status_and_reopens_as_noop() {
    let store = Store::open_in_memory().expect("store");
    let registry = registry();

    let before = store
        .feature_schema_status(FEATURE, &registry)
        .expect("status before apply");
    assert_eq!(before.feature, FEATURE);
    assert_eq!(before.current_version, 0);
    assert!(before.applied.is_empty());
    assert_eq!(
        before
            .pending
            .iter()
            .map(|entry| (entry.version, entry.name.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "create_v1"), (2, "create_v2")]
    );

    store
        .apply_feature_migrations(FEATURE, &registry)
        .expect("apply feature schema");
    let after = store
        .feature_schema_status(FEATURE, &registry)
        .expect("status after apply");
    assert_eq!(after.current_version, 2);
    assert!(after.pending.is_empty());
    assert_eq!(after.applied.len(), 2);
    assert_eq!(after.applied[0].name, "create_v1");
    assert_eq!(after.applied[1].name, "create_v2");
    assert!(
        after
            .applied
            .iter()
            .all(|entry| !entry.applied_at.is_empty())
    );

    // The DDL deliberately omits IF NOT EXISTS. A second call proves the
    // callbacks were skipped rather than merely succeeding idempotently.
    store
        .apply_feature_migrations(FEATURE, &registry)
        .expect("reapply is a no-op");
    assert_eq!(
        store
            .feature_schema_status(FEATURE, &registry)
            .expect("status after reapply"),
        after
    );
}

#[test]
fn failed_feature_migration_rolls_back_its_schema_and_ledger_then_resumes() {
    let store = Store::open_in_memory().expect("store");
    let failing = [
        FeatureMigration::new(1, "create_v1", migration_v1),
        FeatureMigration::new(2, "fails", migration_v2_fails),
    ];

    let error = store
        .apply_feature_migrations(FEATURE, &failing)
        .expect_err("v2 must fail");
    assert!(matches!(error, OrbitError::Migration(_)));
    store
        .with_read_connection(|conn| {
            assert!(table_exists(conn, "feature_test_v1")?);
            assert!(!table_exists(conn, "feature_test_half_applied")?);
            Ok(())
        })
        .expect("inspect rollback");

    let v1_only = [FeatureMigration::new(1, "create_v1", migration_v1)];
    let status = store
        .feature_schema_status(FEATURE, &v1_only)
        .expect("only v1 committed");
    assert_eq!(status.current_version, 1);

    store
        .apply_feature_migrations(FEATURE, &registry())
        .expect("resume with fixed v2");
    assert_eq!(
        store
            .feature_schema_status(FEATURE, &registry())
            .expect("resumed status")
            .current_version,
        2
    );
}

#[test]
fn feature_registry_rejects_non_contiguous_versions_before_running_callbacks() {
    let store = Store::open_in_memory().expect("store");
    let invalid = [
        FeatureMigration::new(1, "create_v1", migration_v1),
        FeatureMigration::new(3, "create_v3", migration_v2),
    ];

    let error = store
        .apply_feature_migrations(FEATURE, &invalid)
        .expect_err("gapped registry must fail");
    assert!(error.to_string().contains("not contiguous"));
    store
        .with_read_connection(|conn| {
            assert!(!table_exists(conn, "feature_test_v1")?);
            Ok(())
        })
        .expect("callback did not run");
}

#[test]
fn feature_ledger_rejects_gaps_changed_names_and_future_versions() {
    let mismatch = Store::open_in_memory().expect("mismatch store");
    mismatch
        .with_transaction(|tx| {
            tx.connection()
                .execute(
                    "INSERT INTO feature_schema_meta(feature, version, name, applied_at)
                     VALUES (?1, 1, 'old_name', '2026-07-18T00:00:00Z')",
                    [FEATURE],
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            Ok(())
        })
        .expect("seed mismatched name");
    let name_error = mismatch
        .feature_schema_status(
            FEATURE,
            &[FeatureMigration::new(1, "create_v1", migration_v1)],
        )
        .expect_err("changed name must fail");
    assert!(name_error.to_string().contains("names are immutable"));

    let gap = Store::open_in_memory().expect("gap store");
    gap.with_transaction(|tx| {
        tx.connection()
            .execute(
                "INSERT INTO feature_schema_meta(feature, version, name, applied_at)
                 VALUES (?1, 2, 'create_v2', '2026-07-18T00:00:00Z')",
                [FEATURE],
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        Ok(())
    })
    .expect("seed ledger gap");
    let gap_error = gap
        .feature_schema_status(FEATURE, &registry())
        .expect_err("gapped ledger must fail");
    assert!(gap_error.to_string().contains("not contiguous"));

    let future = Store::open_in_memory().expect("future store");
    future
        .apply_feature_migrations(FEATURE, &registry())
        .expect("apply through v2");
    let v1_binary = [FeatureMigration::new(1, "create_v1", migration_v1)];
    let future_error = future
        .feature_schema_status(FEATURE, &v1_binary)
        .expect_err("newer feature schema must fail");
    assert!(future_error.to_string().contains("newer"));
    assert!(future_error.to_string().contains("upgrade orbit"));
}

#[test]
fn feature_ledger_rows_are_immutable_and_append_only() {
    let store = Store::open_in_memory().expect("store");
    let v1 = [FeatureMigration::new(1, "create_v1", migration_v1)];
    store
        .apply_feature_migrations(FEATURE, &v1)
        .expect("apply v1");

    store
        .with_transaction(|tx| {
            let update = tx.connection().execute(
                "UPDATE feature_schema_meta SET name = 'changed'
                 WHERE feature = ?1 AND version = 1",
                params![FEATURE],
            );
            assert!(update.is_err(), "ledger update must be rejected");
            Ok(())
        })
        .expect("failed update leaves transaction usable");

    store
        .with_transaction(|tx| {
            let delete = tx.connection().execute(
                "DELETE FROM feature_schema_meta WHERE feature = ?1 AND version = 1",
                params![FEATURE],
            );
            assert!(delete.is_err(), "ledger delete must be rejected");
            Ok(())
        })
        .expect("failed delete leaves transaction usable");

    assert_eq!(
        store
            .feature_schema_status(FEATURE, &v1)
            .expect("ledger preserved")
            .current_version,
        1
    );
}
