//! Cross-workspace isolation for the shared learning envelope index
//! (ORB-10113). The `learnings_index` table is host-global and holds rows for
//! every workspace bound to the same database, so search / upsert / truncate /
//! sync must observe only the runtime's own registered workspace id. These
//! tests back two workspace-local stores with a single SQLite index — the
//! exact shape of the `dk1` multi-workspace ship sweep that surfaced the bug.

use std::path::PathBuf;
use std::sync::{Arc, Barrier};

use tempfile::{TempDir, tempdir};

use super::super::store::LearningFileStore;
use super::test_support::create_params;
use crate::Store;
use crate::backend::LearningSearchParams;

const WS_A: &str = "ws-aaaaaa";
const WS_B: &str = "ws-bbbbbb";

fn store_for(index: &Store, root: PathBuf, workspace_id: &str) -> LearningFileStore {
    LearningFileStore::new_with_index_and_workspace(root, index.clone(), workspace_id)
}

/// Two workspaces sharing one SQLite index, each with its own YAML root, must
/// search and return only their own learning — even when their canonical ids
/// collide (both fresh allocators hand out `L-0001` first).
#[test]
fn search_returns_only_own_workspace_rows_despite_duplicate_ids() {
    let index = Store::open_in_memory().expect("index");
    let dir_a = tempdir().expect("dir a");
    let dir_b = tempdir().expect("dir b");
    let store_a = store_for(&index, dir_a.path().to_path_buf(), WS_A);
    let store_b = store_for(&index, dir_b.path().to_path_buf(), WS_B);

    let a = store_a
        .create_learning(create_params(
            "A workspace rule",
            vec!["shared/**"],
            vec!["alpha"],
        ))
        .expect("create a");
    let b = store_b
        .create_learning(create_params(
            "B workspace rule",
            vec!["shared/**"],
            vec!["beta"],
        ))
        .expect("create b");
    assert_eq!(a.id, "L-0001");
    assert_eq!(
        b.id, "L-0001",
        "the two workspaces must collide on the canonical id for this test to be meaningful"
    );

    // Path search: each store sees only its own summary and scope, never the
    // other workspace's row that happens to share the id.
    let hits_a = store_a
        .search_learnings(LearningSearchParams {
            path: Some("shared/x.rs".to_string()),
            ..Default::default()
        })
        .expect("search a");
    assert_eq!(hits_a.len(), 1);
    assert_eq!(hits_a[0].learning.summary, "A workspace rule");
    assert_eq!(hits_a[0].learning.scope.tags, vec!["alpha"]);

    let hits_b = store_b
        .search_learnings(LearningSearchParams {
            path: Some("shared/x.rs".to_string()),
            ..Default::default()
        })
        .expect("search b");
    assert_eq!(hits_b.len(), 1);
    assert_eq!(hits_b[0].learning.summary, "B workspace rule");
    assert_eq!(hits_b[0].learning.scope.tags, vec!["beta"]);

    // Tag search: neither workspace's tag matches in the other.
    let beta_in_a = store_a
        .search_learnings(LearningSearchParams {
            tag: Some("beta".to_string()),
            ..Default::default()
        })
        .expect("beta in a");
    assert!(
        beta_in_a.is_empty(),
        "workspace A must not see workspace B's tag"
    );
    let alpha_in_b = store_b
        .search_learnings(LearningSearchParams {
            tag: Some("alpha".to_string()),
            ..Default::default()
        })
        .expect("alpha in b");
    assert!(
        alpha_in_b.is_empty(),
        "workspace B must not see workspace A's tag"
    );
}

/// Syncing workspace A truncates and rebuilds only A's rows; workspace B's row
/// is never deleted or replaced (no last-writer-wins across workspaces).
#[test]
fn sequential_sync_leaves_other_workspace_rows_intact() {
    let index = Store::open_in_memory().expect("index");
    let dir_a = tempdir().expect("dir a");
    let dir_b = tempdir().expect("dir b");
    let store_a = store_for(&index, dir_a.path().to_path_buf(), WS_A);
    let store_b = store_for(&index, dir_b.path().to_path_buf(), WS_B);

    store_a
        .create_learning(create_params("A one", vec!["a/**"], vec![]))
        .expect("a one");
    store_b
        .create_learning(create_params("B one", vec!["b/**"], vec![]))
        .expect("b one");

    store_a.sync_learnings().expect("sync a");
    let a_rows = index.list_active_learning_rows(WS_A).expect("a rows");
    assert_eq!(a_rows.len(), 1);
    assert_eq!(a_rows[0].summary, "A one");
    let b_rows = index.list_active_learning_rows(WS_B).expect("b rows");
    assert_eq!(b_rows.len(), 1, "syncing A must not disturb B's rows");
    assert_eq!(b_rows[0].summary, "B one");

    store_b.sync_learnings().expect("sync b");
    let a_rows = index
        .list_active_learning_rows(WS_A)
        .expect("a rows after b sync");
    assert_eq!(a_rows.len(), 1, "syncing B must not disturb A's rows");
    assert_eq!(a_rows[0].summary, "A one");
    let b_rows = index
        .list_active_learning_rows(WS_B)
        .expect("b rows after b sync");
    assert_eq!(b_rows.len(), 1);
    assert_eq!(b_rows[0].summary, "B one");
}

/// Concurrent, repeated syncs of two workspaces against the same database
/// converge to each workspace's own rows — no cross-workspace deletion,
/// duplication, or replacement.
#[test]
fn concurrent_sync_cannot_cross_contaminate_workspaces() {
    let index = Store::open_in_memory().expect("index");
    let dir_a: TempDir = tempdir().expect("dir a");
    let dir_b: TempDir = tempdir().expect("dir b");
    let store_a = Arc::new(store_for(&index, dir_a.path().to_path_buf(), WS_A));
    let store_b = Arc::new(store_for(&index, dir_b.path().to_path_buf(), WS_B));

    for i in 0..5 {
        store_a
            .create_learning(create_params(&format!("A {i}"), vec!["a/**"], vec![]))
            .expect("a create");
        store_b
            .create_learning(create_params(&format!("B {i}"), vec!["b/**"], vec![]))
            .expect("b create");
    }

    let barrier = Arc::new(Barrier::new(2));
    let sa = Arc::clone(&store_a);
    let ba = Arc::clone(&barrier);
    let handle_a = std::thread::spawn(move || {
        ba.wait();
        for _ in 0..10 {
            sa.sync_learnings().expect("sync a");
        }
    });
    let sb = Arc::clone(&store_b);
    let bb = barrier;
    let handle_b = std::thread::spawn(move || {
        bb.wait();
        for _ in 0..10 {
            sb.sync_learnings().expect("sync b");
        }
    });
    handle_a.join().expect("join a");
    handle_b.join().expect("join b");

    let a_rows = index.list_active_learning_rows(WS_A).expect("a rows");
    assert_eq!(
        a_rows.len(),
        5,
        "workspace A must retain exactly its own rows"
    );
    assert!(
        a_rows.iter().all(|row| row.summary.starts_with("A ")),
        "no workspace B row may appear under workspace A"
    );
    let b_rows = index.list_active_learning_rows(WS_B).expect("b rows");
    assert_eq!(
        b_rows.len(),
        5,
        "workspace B must retain exactly its own rows"
    );
    assert!(
        b_rows.iter().all(|row| row.summary.starts_with("B ")),
        "no workspace A row may appear under workspace B"
    );
}
