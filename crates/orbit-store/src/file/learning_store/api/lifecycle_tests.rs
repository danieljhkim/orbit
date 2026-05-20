//! Lifecycle (supersede/archive/delete) tests split per ORB-00116.

use orbit_common::types::LearningStatus;
use tempfile::tempdir;

use super::super::record::read_learning_file;
use super::store::LearningFileStore;
use super::test_support::{create_params, legacy_learning_yaml, store_with_index};
use crate::Store;

#[test]
fn supersession_moves_yaml_and_updates_both_records() {
    let (_dir, store) = store_with_index();
    let old = store
        .create_learning(create_params("old", vec![], vec![]))
        .expect("old");
    let new = store
        .create_learning(create_params("new", vec![], vec![]))
        .expect("new");

    store
        .supersede_learning(&old.id, &new.id)
        .expect("supersede");

    let loaded_old = store
        .get_learning(&old.id)
        .expect("get old")
        .expect("present");
    let loaded_new = store
        .get_learning(&new.id)
        .expect("get new")
        .expect("present");

    assert_eq!(loaded_old.status, LearningStatus::Superseded);
    assert_eq!(loaded_old.superseded_by.as_deref(), Some(new.id.as_str()));
    assert_eq!(loaded_new.supersedes.as_deref(), Some(old.id.as_str()));
    assert_eq!(loaded_new.status, LearningStatus::Active);

    // Both index rows reflect new state.
    let index = store.index.as_ref().expect("index");
    let old_row = index
        .get_learning_index_row(&old.id)
        .expect("query old")
        .expect("present");
    let new_row = index
        .get_learning_index_row(&new.id)
        .expect("query new")
        .expect("present");
    assert_eq!(old_row.status, LearningStatus::Superseded);
    assert_eq!(new_row.status, LearningStatus::Active);
}

#[test]
fn reindex_rebuilds_index_from_yaml() {
    let (_dir, store) = store_with_index();
    let _a = store
        .create_learning(create_params("a", vec!["a/**"], vec!["x"]))
        .expect("a");
    let _b = store
        .create_learning(create_params("b", vec!["b/**"], vec!["y"]))
        .expect("b");

    // Wipe the index from under the store.
    store
        .index
        .as_ref()
        .expect("index")
        .truncate_learning_index()
        .expect("truncate");
    let active = store
        .index
        .as_ref()
        .expect("index")
        .list_active_learning_rows()
        .expect("list");
    assert!(active.is_empty());

    store.reindex_learnings().expect("reindex");
    let active = store
        .index
        .as_ref()
        .expect("index")
        .list_active_learning_rows()
        .expect("list");
    assert_eq!(active.len(), 2);
}

#[test]
fn migrate_layout_preserves_list_parity_and_reindex_projection() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("learnings");
    std::fs::create_dir_all(root.join("superseded")).expect("legacy dirs");
    std::fs::write(
        root.join("L-0001.yaml"),
        legacy_learning_yaml("L-0001", "active", "Active rule", 10),
    )
    .expect("active yaml");
    std::fs::write(
        root.join("superseded").join("L-0002.yaml"),
        legacy_learning_yaml("L-0002", "superseded", "Old rule", 20),
    )
    .expect("superseded yaml");

    let legacy_active = read_learning_file(&root.join("L-0001.yaml")).expect("active");
    let legacy_superseded =
        read_learning_file(&root.join("superseded").join("L-0002.yaml")).expect("superseded");
    let index = Store::open_in_memory().expect("index");
    index
        .upsert_learning_index_row(&legacy_active)
        .expect("index active");
    index
        .upsert_learning_index_row(&legacy_superseded)
        .expect("index superseded");
    let before_active_row = index
        .get_learning_index_row("L-0001")
        .expect("row active")
        .expect("present");
    let before_superseded_row = index
        .get_learning_index_row("L-0002")
        .expect("row superseded")
        .expect("present");

    super::super::migration::migrate_learning_layout(&root, dir.path()).expect("migrate");
    let store = LearningFileStore::new_with_index(root.clone(), index.clone());
    store.reindex_learnings().expect("reindex");

    let active = store
        .list_learnings(Some(LearningStatus::Active))
        .expect("list active");
    let superseded = store
        .list_learnings(Some(LearningStatus::Superseded))
        .expect("list superseded");
    assert_eq!(active, vec![legacy_active]);
    assert_eq!(superseded, vec![legacy_superseded]);
    assert_eq!(
        index
            .get_learning_index_row("L-0001")
            .expect("row active after")
            .expect("present"),
        before_active_row
    );
    assert_eq!(
        index
            .get_learning_index_row("L-0002")
            .expect("row superseded after")
            .expect("present"),
        before_superseded_row
    );
}
