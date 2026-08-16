use std::fs;

use orbit_core::config::load_effective_config;

use super::super::show::effective_json;
use super::test_runtime;

#[test]
fn effective_json_attributes_each_merged_value_to_its_source() {
    let (_root, runtime, global_root, workspace_root) = test_runtime();
    fs::write(
        global_root.join("config.toml"),
        r#"
[workflow]
base_branch = "integration"

[execution.codex]
sandbox = "danger-full-access"
"#,
    )
    .expect("write global config");
    fs::write(
        workspace_root.join("config.toml"),
        "[scoring]\nenabled = false\n",
    )
    .expect("write workspace config");

    let effective = load_effective_config(&global_root, &workspace_root)
        .expect("load effective layered config");
    let json = effective_json(&runtime, effective.values());

    assert_eq!(json["settings"]["scoring.enabled"], false);
    assert_eq!(json["provenance"]["scoring.enabled"]["scope"], "workspace");
    assert_eq!(
        json["provenance"]["workflow.base_branch"]["scope"],
        "global"
    );
    assert_eq!(
        json["provenance"]["execution.codex.sandbox"]["scope"],
        "built-in"
    );
    assert_eq!(
        json["provenance"]["workflow.base_branch"]["path"],
        global_root.join("config.toml").to_string_lossy().as_ref()
    );
}
