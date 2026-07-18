use super::super::host::{HostMode, inspect_host_identity};

#[test]
fn compatibility_module_reexports_host_identity_surface() {
    let root = tempfile::tempdir().expect("tempdir");
    assert_eq!(
        HostMode::parse("standalone").expect("mode"),
        HostMode::Standalone
    );
    assert!(
        inspect_host_identity(root.path())
            .expect("inspect")
            .host_id()
            .is_none()
    );
}
