use std::collections::BTreeSet;

use toml::Value;

const MANIFEST: &str = include_str!("../Cargo.toml");

#[test]
fn only_orbit_common_is_an_internal_dependency() {
    let manifest = parse_manifest();
    let mut dependency_names = BTreeSet::new();

    collect_dependencies(&manifest, &mut dependency_names);

    let orbit_deps = dependency_names
        .iter()
        .filter(|name| name.starts_with("orbit-"))
        .cloned()
        .collect::<Vec<_>>();

    assert_eq!(
        orbit_deps,
        vec!["orbit-common".to_string()],
        "orbit-registry must remain consumer-agnostic and depend only on orbit-common internally"
    );

    for forbidden in [
        "orbit-knowledge",
        "orbit-store",
        "orbit-tools",
        "orbit-policy",
        "orbit-exec",
    ] {
        assert!(
            !dependency_names.contains(forbidden),
            "forbidden internal dependency added: {forbidden}"
        );
    }
}

#[test]
fn git2_transport_is_feature_gated() {
    let manifest = parse_manifest();
    let dependencies = manifest
        .get("dependencies")
        .and_then(Value::as_table)
        .expect("dependencies table");
    let git2_dependency = dependencies
        .get("git2")
        .and_then(Value::as_table)
        .expect("git2 dependency table");

    assert_eq!(
        git2_dependency.get("optional").and_then(Value::as_bool),
        Some(true),
        "git2 must stay optional"
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
        features
            .get("transport-git2")
            .and_then(Value::as_array)
            .is_some_and(|values| values
                .iter()
                .any(|value| value.as_str() == Some("dep:git2"))),
        "transport-git2 must gate the optional git2 dependency"
    );
}

fn parse_manifest() -> Value {
    toml::from_str(MANIFEST).expect("crate manifest parses")
}

fn collect_dependencies(manifest: &Value, names: &mut BTreeSet<String>) {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        collect_dependency_table(manifest.get(section), names);
    }

    if let Some(targets) = manifest.get("target").and_then(Value::as_table) {
        for target in targets.values() {
            for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
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
