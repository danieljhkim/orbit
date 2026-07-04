//! Sibling tests for the workspace-layout migration registry (ORB-10012).

use std::fs;
use std::path::Path;

use orbit_common::types::OrbitError;

use super::{
    LAYOUT_MIGRATIONS, LayoutMigration, SUPPORTED_LAYOUT_VERSION, current_layout_version,
    pending_layout_migrations, pending_with, upgrade_with, upgrade_workspace_layout,
};

fn temp_orbit_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("create temp .orbit dir")
}

fn marker_contents(orbit_dir: &Path) -> String {
    fs::read_to_string(orbit_dir.join("state").join("layout.version")).expect("read marker")
}

// ── shipping registry ──

#[test]
fn shipping_registry_is_strictly_increasing_and_matches_supported_version() {
    let mut previous = 0u32;
    for migration in LAYOUT_MIGRATIONS {
        assert!(
            migration.version > previous,
            "registry must be strictly increasing: v{} after v{previous}",
            migration.version
        );
        previous = migration.version;
    }
    assert_eq!(
        previous, SUPPORTED_LAYOUT_VERSION,
        "SUPPORTED_LAYOUT_VERSION must equal the newest registry entry"
    );
}

#[test]
fn fresh_workspace_adopts_the_baseline_and_stamps_the_marker() {
    let temp = temp_orbit_dir();

    assert_eq!(current_layout_version(temp.path()).expect("version"), 0);
    let pending = pending_layout_migrations(temp.path()).expect("pending");
    assert_eq!(pending.len(), LAYOUT_MIGRATIONS.len());
    assert_eq!(pending[0].name, "baseline");

    let report = upgrade_workspace_layout(temp.path()).expect("upgrade");
    assert_eq!(report.from_version, 0);
    assert_eq!(report.to_version, SUPPORTED_LAYOUT_VERSION);
    assert_eq!(report.applied.len(), LAYOUT_MIGRATIONS.len());
    assert_eq!(marker_contents(temp.path()).trim(), "1");
    assert_eq!(
        current_layout_version(temp.path()).expect("version"),
        SUPPORTED_LAYOUT_VERSION
    );
}

#[test]
fn current_workspace_is_a_no_op_with_no_pending_migrations() {
    let temp = temp_orbit_dir();
    upgrade_workspace_layout(temp.path()).expect("first upgrade");

    let report = upgrade_workspace_layout(temp.path()).expect("second upgrade");
    assert_eq!(report.from_version, SUPPORTED_LAYOUT_VERSION);
    assert_eq!(report.to_version, SUPPORTED_LAYOUT_VERSION);
    assert!(report.applied.is_empty());
    assert!(
        pending_layout_migrations(temp.path())
            .expect("pending")
            .is_empty()
    );
}

#[test]
fn newer_marker_refuses_with_downgrade_guard() {
    let temp = temp_orbit_dir();
    fs::create_dir_all(temp.path().join("state")).expect("mkdir state");
    fs::write(temp.path().join("state").join("layout.version"), "99\n").expect("write marker");

    let error = upgrade_workspace_layout(temp.path()).expect_err("must refuse newer layout");
    assert!(matches!(error, OrbitError::Migration(_)), "{error:?}");
    let message = error.to_string();
    assert!(message.contains("layout version 99"), "{message}");
    assert!(message.contains("upgrade orbit"), "{message}");

    // The marker is untouched and the pre-flight remains inspectable.
    assert_eq!(current_layout_version(temp.path()).expect("version"), 99);
    assert!(
        pending_layout_migrations(temp.path())
            .expect("pending")
            .is_empty()
    );
}

#[test]
fn corrupt_marker_is_a_typed_error_naming_the_file() {
    let temp = temp_orbit_dir();
    fs::create_dir_all(temp.path().join("state")).expect("mkdir state");
    fs::write(temp.path().join("state").join("layout.version"), "banana").expect("write marker");

    let error = upgrade_workspace_layout(temp.path()).expect_err("must refuse corrupt marker");
    let message = error.to_string();
    assert!(message.contains("layout.version"), "{message}");
    assert!(message.contains("banana"), "{message}");
}

// ── test-only v2 registry: exercises a real layout change end to end ──

fn toy_v2_apply(orbit_dir: &Path) -> Result<(), OrbitError> {
    // Idempotent rename: move legacy `notes.txt` under `notes/` (staged
    // write-new-then-swap shape a real migration would use).
    let legacy = orbit_dir.join("notes.txt");
    let target_dir = orbit_dir.join("notes");
    fs::create_dir_all(&target_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    if legacy.exists() {
        fs::rename(&legacy, target_dir.join("notes.txt"))
            .map_err(|e| OrbitError::Io(e.to_string()))?;
    }
    Ok(())
}

fn failing_apply(_orbit_dir: &Path) -> Result<(), OrbitError> {
    Err(OrbitError::Execution("boom".to_string()))
}

const TOY_V2_REGISTRY: &[LayoutMigration] = &[
    LayoutMigration {
        version: 1,
        name: "baseline",
        description: "adopt the versioned layout",
        apply: |_| Ok(()),
    },
    LayoutMigration {
        version: 2,
        name: "notes-into-subdir",
        description: "move notes.txt under notes/",
        apply: toy_v2_apply,
    },
];

#[test]
fn toy_v2_migration_applies_in_order_and_advances_the_marker() {
    let temp = temp_orbit_dir();
    fs::write(temp.path().join("notes.txt"), "hello").expect("write legacy file");

    let pending = pending_with(temp.path(), TOY_V2_REGISTRY).expect("pending");
    assert_eq!(
        pending.iter().map(|m| m.version).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(pending[1].description, "move notes.txt under notes/");

    let report = upgrade_with(temp.path(), TOY_V2_REGISTRY).expect("upgrade");
    assert_eq!(report.from_version, 0);
    assert_eq!(report.to_version, 2);
    assert_eq!(
        report
            .applied
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>(),
        vec!["baseline", "notes-into-subdir"]
    );
    assert_eq!(marker_contents(temp.path()).trim(), "2");
    assert!(!temp.path().join("notes.txt").exists());
    assert_eq!(
        fs::read_to_string(temp.path().join("notes").join("notes.txt")).expect("moved file"),
        "hello"
    );

    // Idempotent rerun: nothing pending, nothing re-applied.
    let rerun = upgrade_with(temp.path(), TOY_V2_REGISTRY).expect("rerun");
    assert!(rerun.applied.is_empty());
}

#[test]
fn upgrade_applies_only_migrations_newer_than_the_marker() {
    let temp = temp_orbit_dir();
    // Already on v1: only the v2 entry should run.
    upgrade_workspace_layout(temp.path()).expect("adopt baseline");
    fs::write(temp.path().join("notes.txt"), "hello").expect("write legacy file");

    let report = upgrade_with(temp.path(), TOY_V2_REGISTRY).expect("upgrade");
    assert_eq!(report.from_version, 1);
    assert_eq!(report.to_version, 2);
    assert_eq!(report.applied.len(), 1);
    assert_eq!(report.applied[0].name, "notes-into-subdir");
}

#[test]
fn failed_migration_keeps_the_marker_at_the_last_applied_version() {
    let temp = temp_orbit_dir();
    const FAILING_REGISTRY: &[LayoutMigration] = &[
        LayoutMigration {
            version: 1,
            name: "baseline",
            description: "adopt",
            apply: |_| Ok(()),
        },
        LayoutMigration {
            version: 2,
            name: "explodes",
            description: "always fails",
            apply: failing_apply,
        },
    ];

    let error = upgrade_with(temp.path(), FAILING_REGISTRY).expect_err("v2 must fail");
    let message = error.to_string();
    assert!(message.contains("v2"), "{message}");
    assert!(message.contains("explodes"), "{message}");
    // v1 landed and was recorded; the failed v2 did not advance the marker,
    // so a fixed binary resumes exactly at v2.
    assert_eq!(current_layout_version(temp.path()).expect("version"), 1);

    let report = upgrade_with(temp.path(), TOY_V2_REGISTRY).expect("resume with fixed registry");
    assert_eq!(report.from_version, 1);
    assert_eq!(report.applied.len(), 1);
    assert_eq!(report.applied[0].version, 2);
}

#[test]
fn non_increasing_registry_is_rejected() {
    let temp = temp_orbit_dir();
    const BROKEN_REGISTRY: &[LayoutMigration] = &[
        LayoutMigration {
            version: 2,
            name: "two",
            description: "",
            apply: |_| Ok(()),
        },
        LayoutMigration {
            version: 2,
            name: "two-again",
            description: "",
            apply: |_| Ok(()),
        },
    ];

    let error = upgrade_with(temp.path(), BROKEN_REGISTRY).expect_err("must reject registry");
    assert!(error.to_string().contains("strictly increasing"), "{error}");
    let error = pending_with(temp.path(), BROKEN_REGISTRY).expect_err("must reject registry");
    assert!(error.to_string().contains("strictly increasing"), "{error}");
}
