use super::*;

#[test]
fn host_qualified_selector_accepts_only_machine_and_workspace_ids() {
    let selector: HostQualifiedSelector = "hm_alpha-1/ws_orbit"
        .parse()
        .expect("host-qualified selector");

    assert_eq!(selector.machine_id(), "hm_alpha-1");
    assert_eq!(selector.workspace_id(), "ws_orbit");
    assert_eq!(selector.to_string(), "hm_alpha-1/ws_orbit");
}

#[test]
fn host_qualified_selector_rejects_bare_workspace_id() {
    assert!(matches!(
        "ws_orbit".parse::<HostQualifiedSelector>(),
        Err(OrbitError::UnknownSelector(_))
    ));
}

#[test]
fn host_qualified_selector_rejects_display_host_name() {
    assert!(matches!(
        "orbit-linux/ws_orbit".parse::<HostQualifiedSelector>(),
        Err(OrbitError::UnknownSelector(_))
    ));
}

#[test]
fn host_qualified_selector_rejects_paths() {
    for token in [
        "/srv/orbit/ws_orbit",
        "hm_alpha/../ws_orbit",
        "hm_alpha/C:\\orbit",
    ] {
        assert!(
            matches!(
                token.parse::<HostQualifiedSelector>(),
                Err(OrbitError::UnknownSelector(_))
            ),
            "unexpectedly accepted {token}"
        );
    }
}

#[test]
fn distinct_machine_destinations_load() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = destinations_path(root.path());
    std::fs::write(
        &path,
        r#"
[[destinations]]
ssh = "orbit-a"
machine_id = "hm_alpha"

[[destinations]]
ssh = "operator@orbit-b"
machine_id = "hm_beta"
"#,
    )
    .expect("write destinations");

    let loaded = load_destinations(&path).expect("load destinations");
    assert_eq!(loaded.destinations.len(), 2);
    assert_eq!(loaded.destinations[0].ssh, "orbit-a");
    assert_eq!(loaded.destinations[0].machine_id, "hm_alpha");
    assert_eq!(loaded.destinations[1].machine_id, "hm_beta");
}

#[test]
fn duplicate_machine_destinations_fail_at_load() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = destinations_path(root.path());
    std::fs::write(
        &path,
        r#"
[[destinations]]
ssh = "orbit-a"
machine_id = "hm_duplicate"

[[destinations]]
ssh = "orbit-b"
machine_id = "hm_duplicate"
"#,
    )
    .expect("write destinations");

    let error = load_destinations(&path).expect_err("duplicate must fail at config load");
    assert!(matches!(error, OrbitError::AmbiguousDestination(_)));
}

#[test]
fn destinations_require_machine_id() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = destinations_path(root.path());
    std::fs::write(
        &path,
        r#"
[[destinations]]
ssh = "orbit-a"
"#,
    )
    .expect("write destinations");

    let error = load_destinations(&path).expect_err("machine_id is required");
    assert!(matches!(error, OrbitError::InvalidInput(_)));
}
