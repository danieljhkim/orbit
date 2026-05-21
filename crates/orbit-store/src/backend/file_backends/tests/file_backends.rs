// Migrated from backend/file_backends.rs per ORB-00231
use std::path::PathBuf;

use super::super::*;

#[test]
fn adr_file_store_returns_workspace_only_strategy() {
    let store = AdrFileStore::new(PathBuf::from("/tmp/unused-adr-root"));
    assert_eq!(
        ScopedStore::<Adr>::strategy(&store),
        ScopeStrategy::WorkspaceOnly
    );
}
