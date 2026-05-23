// Migrated from file/learning_store/migration.rs per ORB-00231
use std::fs;

use fs2::FileExt;
use tempfile::tempdir;

use super::super::*;

#[test]
fn migration_moves_flat_active_and_superseded_without_touching_tags() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("learnings");
    fs::create_dir_all(root.join("superseded")).expect("dirs");
    fs::write(root.join("tags.yaml"), "tags:\n  rust: {}\n").expect("tags");
    fs::write(root.join("L-0001.yaml"), "id: L-0001\nstatus: active\ncreated_at: 2026-05-17T00:00:00Z\nupdated_at: 2026-05-17T00:00:00Z\n").expect("active");
    fs::write(root.join("superseded").join("L-0002.yaml"), "id: L-0002\nstatus: superseded\ncreated_at: 2026-05-17T00:00:00Z\nupdated_at: 2026-05-17T00:00:00Z\n").expect("superseded");
    let tags_before = fs::read(root.join("tags.yaml")).expect("read tags");

    let report = migrate_learning_layout(&root, dir.path()).expect("migrate");

    assert_eq!(report.moved_active, 1);
    assert_eq!(report.moved_superseded, 1);
    assert!(report.removed_superseded_dir);
    assert!(!root.join("L-0001.yaml").exists());
    assert!(!root.join("superseded").exists());
    assert!(root.join("L-0001").join("learning.yaml").is_file());
    assert!(root.join("L-0002").join("learning.yaml").is_file());
    assert_eq!(
        fs::read(root.join("tags.yaml")).expect("read tags"),
        tags_before
    );
}

#[test]
fn migration_is_noop_on_per_entity_layout() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("learnings");
    fs::create_dir_all(root.join("L-0001")).expect("dirs");
    fs::write(root.join("L-0001").join("learning.yaml"), "").expect("learning");

    let report = migrate_learning_layout(&root, dir.path()).expect("migrate");

    assert!(report.already_migrated);
    assert_eq!(report.moved_total(), 0);
    assert!(!dir.path().join("state").join("workspace.lock").exists());
}

#[test]
fn migration_refuses_when_workspace_lock_is_held() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("learnings");
    fs::create_dir_all(&root).expect("dirs");
    fs::write(root.join("L-0001.yaml"), "").expect("legacy");
    let lock_path = dir.path().join(WORKSPACE_LOCK_RELATIVE_PATH);
    fs::create_dir_all(lock_path.parent().expect("lock parent")).expect("lock dir");
    let mut lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("open lock");
    writeln!(lock, "pid=12345").expect("write owner");
    lock.lock_exclusive().expect("hold lock");

    let err = migrate_learning_layout(&root, dir.path()).expect_err("must refuse");

    assert!(matches!(err, OrbitError::WorkspaceError(_)));
    assert!(err.to_string().contains("process pid=12345"));
}

#[test]
fn legacy_flat_layout_detection_names_migration_command() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("learnings");
    fs::create_dir_all(&root).expect("dirs");
    fs::write(root.join("L-0001.yaml"), "").expect("legacy");

    let err = reject_legacy_flat_layout(&root).expect_err("legacy rejected");

    assert!(matches!(err, OrbitError::Migration(_)));
    assert!(err.to_string().contains("orbit learning migrate-layout"));
}
