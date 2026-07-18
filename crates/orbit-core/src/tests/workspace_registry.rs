use orbit_common::types::WorkspaceRegistry;

use super::{find_workspace, registry_path_for};

#[test]
fn compatibility_module_reexports_workspace_registry_surface() {
    let root = tempfile::tempdir().expect("tempdir");
    assert_eq!(
        registry_path_for(root.path()),
        root.path().join("workspaces.json")
    );
    assert!(find_workspace(&WorkspaceRegistry::default(), "missing").is_none());
}
