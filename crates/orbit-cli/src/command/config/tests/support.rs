use std::fs;

use orbit_core::config::ConfigScope;

use super::super::support::{ConfigScopeArg, resolve_scope};
use super::test_runtime;

#[test]
fn effective_scope_resolves_to_global_when_no_workspace_config() {
    let (_root, runtime, global_root, _workspace_root) = test_runtime();
    fs::write(
        global_root.join("config.toml"),
        "[workflow]\nbase_branch = \"main\"\n",
    )
    .expect("write global config");

    let (scope, path) = resolve_scope(&runtime, ConfigScopeArg::Effective);
    assert_eq!(scope, ConfigScope::Global);
    assert_eq!(path, global_root.join("config.toml"));
}

#[test]
fn effective_scope_resolves_to_workspace_when_it_exists() {
    let (_root, runtime, global_root, workspace_root) = test_runtime();
    fs::write(
        global_root.join("config.toml"),
        "[workflow]\nbase_branch = \"main\"\n",
    )
    .expect("write global config");
    fs::write(
        workspace_root.join("config.toml"),
        "[workflow]\nbase_branch = \"agent-main\"\n",
    )
    .expect("write workspace config");

    let (scope, path) = resolve_scope(&runtime, ConfigScopeArg::Effective);
    assert_eq!(scope, ConfigScope::Workspace);
    assert_eq!(path, workspace_root.join("config.toml"));
}

#[test]
fn explicit_global_scope_ignores_workspace_config() {
    let (_root, runtime, global_root, workspace_root) = test_runtime();
    fs::write(
        workspace_root.join("config.toml"),
        "[workflow]\nbase_branch = \"agent-main\"\n",
    )
    .expect("write workspace config");

    let (scope, path) = resolve_scope(&runtime, ConfigScopeArg::Global);
    assert_eq!(scope, ConfigScope::Global);
    assert_eq!(path, global_root.join("config.toml"));
}

#[test]
fn explicit_workspace_scope_reads_workspace_path_even_when_absent() {
    let (_root, runtime, _global_root, workspace_root) = test_runtime();

    let (scope, path) = resolve_scope(&runtime, ConfigScopeArg::Workspace);
    assert_eq!(scope, ConfigScope::Workspace);
    assert_eq!(path, workspace_root.join("config.toml"));
}
