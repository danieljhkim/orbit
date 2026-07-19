#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;

use toml::Value;

const MANIFEST: &str = include_str!("../Cargo.toml");

#[test]
fn vertical_feature_depends_only_on_approved_runtime_layers() {
    // ADR-0240: Remote composes neutral Core kernels, Store persistence, and
    // the generic MCP contract; none of those lower layers depends back on
    // this feature crate.
    let manifest = parse_manifest();
    let mut dependency_names = BTreeSet::new();

    collect_dependencies(
        &manifest,
        &["dependencies", "build-dependencies"],
        &mut dependency_names,
    );

    let orbit_deps = dependency_names
        .iter()
        .filter(|name| name.starts_with("orbit-"))
        .cloned()
        .collect::<Vec<_>>();

    assert_eq!(
        orbit_deps,
        vec![
            "orbit-common".to_string(),
            "orbit-core".to_string(),
            "orbit-mcp".to_string(),
            "orbit-store".to_string(),
        ],
        "orbit-remote may compose only the approved runtime layers in this cut"
    );

    for forbidden in ["orbit-cmd", "orbit-tools", "orbit-policy", "orbit-exec"] {
        assert!(
            !dependency_names.contains(forbidden),
            "forbidden internal dependency added: {forbidden}"
        );
    }
}

#[test]
fn command_layer_dependency_is_test_only_for_the_cross_layer_canary() {
    let manifest = parse_manifest();
    let mut dependency_names = BTreeSet::new();

    collect_dependencies(&manifest, &["dev-dependencies"], &mut dependency_names);

    let orbit_deps = dependency_names
        .iter()
        .filter(|name| name.starts_with("orbit-"))
        .cloned()
        .collect::<Vec<_>>();

    assert_eq!(
        orbit_deps,
        vec!["orbit-cmd".to_string()],
        "orbit-cmd is permitted only as the explicit cross-layer learning-state canary"
    );
}

#[test]
fn manifest_does_not_expose_dead_git2_transport() {
    let manifest = parse_manifest();
    let dependencies = manifest
        .get("dependencies")
        .and_then(Value::as_table)
        .expect("dependencies table");
    assert!(
        !dependencies.contains_key("git2"),
        "dead git2 transport dependency must stay removed"
    );

    let features = manifest
        .get("features")
        .and_then(Value::as_table)
        .expect("features table");
    assert!(
        features
            .get("default")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty),
        "default features must not enable git2"
    );
    assert!(
        !features.contains_key("transport-git2"),
        "dead transport-git2 feature must stay removed"
    );
}

fn parse_manifest() -> Value {
    toml::from_str(MANIFEST).expect("crate manifest parses")
}

fn collect_dependencies(manifest: &Value, sections: &[&str], names: &mut BTreeSet<String>) {
    for section in sections {
        collect_dependency_table(manifest.get(section), names);
    }

    if let Some(targets) = manifest.get("target").and_then(Value::as_table) {
        for target in targets.values() {
            for section in sections {
                collect_dependency_table(target.get(section), names);
            }
        }
    }
}

fn collect_dependency_table(section: Option<&Value>, names: &mut BTreeSet<String>) {
    let Some(table) = section.and_then(Value::as_table) else {
        return;
    };

    for (name, value) in table {
        let package_name = value
            .as_table()
            .and_then(|table| table.get("package"))
            .and_then(Value::as_str)
            .unwrap_or(name);
        names.insert(package_name.to_string());
    }
}
