use clap::Args;
use orbit_common::utility::redaction::redact_home_dir;
use orbit_core::config::ConfigScope;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Map, Value as JsonValue, json};

use crate::command::Execute;

use super::support::{ConfigScopeArg, global_config_path, open_store_for_scope};

#[derive(Args)]
pub struct ConfigShowArgs {
    #[arg(long, value_enum, default_value_t = ConfigScopeArg::Effective)]
    pub scope: ConfigScopeArg,
    #[arg(long)]
    pub json: bool,
}

impl Execute for ConfigShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let store = open_store_for_scope(runtime, self.scope)?;
        let snapshot = store.snapshot()?;
        let settings = snapshot.all_values();

        // Both files exist and this is the *effective* view (i.e. we
        // resolved to the workspace file): global is shadowed entirely
        // under replace-not-merge, so flag it — otherwise a user could read
        // a stale global config.toml edit and wonder why it has no effect.
        let global_path = global_config_path(runtime);
        let shadowed_global_path = (self.scope == ConfigScopeArg::Effective
            && store.scope() == ConfigScope::Workspace
            && global_path.exists())
        .then_some(global_path);

        if self.json {
            print_json(
                runtime,
                &store,
                &snapshot,
                &settings,
                shadowed_global_path.as_deref(),
            )
        } else {
            print_text(
                runtime,
                &store,
                &snapshot,
                &settings,
                shadowed_global_path.as_deref(),
            );
            Ok(())
        }
    }
}

fn print_json(
    runtime: &OrbitRuntime,
    store: &orbit_core::config::ConfigStore,
    snapshot: &orbit_core::config::ConfigSnapshot,
    settings: &[(&'static str, JsonValue)],
    shadowed_global_path: Option<&std::path::Path>,
) -> Result<(), OrbitError> {
    let mut settings_obj = Map::new();
    for (key, value) in settings {
        settings_obj.insert((*key).to_string(), value.clone());
    }

    // `global_root`/`shared_root`/`local_root`/`config_path`/`persistence`
    // stay top-level (not nested under a `derived` object) because an
    // existing contract test (`worktree_resolution.rs`) already asserts on
    // `shared_root`/`local_root` at the top level of `config show --json`,
    // and explicitly forbids reintroducing renamed aliases for them. The
    // `derived:` grouping the task asks for is rendered in the human-
    // readable text output below; the JSON shape keeps these pre-existing
    // field names and positions unchanged.
    crate::output::json::print_pretty(&json!({
        "source": {
            "scope": store.scope().label(),
            "path": store.path().to_string_lossy(),
        },
        "shadowed_global_path": shadowed_global_path.map(|p| p.to_string_lossy().into_owned()),
        "settings": settings_obj,
        "execution_env_inherit": snapshot.execution_env_inherit,
        "global_root": runtime.global_root().to_string_lossy(),
        "shared_root": runtime.shared_root().to_string_lossy(),
        "local_root": runtime.local_root().to_string_lossy(),
        "config_path": store.path().to_string_lossy(),
        "persistence": runtime.persistence_config_json(),
    }))
}

fn print_text(
    runtime: &OrbitRuntime,
    store: &orbit_core::config::ConfigStore,
    snapshot: &orbit_core::config::ConfigSnapshot,
    settings: &[(&'static str, JsonValue)],
    shadowed_global_path: Option<&std::path::Path>,
) {
    println!(
        "source: {} ({})",
        store.scope().label(),
        redact_home_dir(&store.path().display().to_string())
    );
    if let Some(global_path) = shadowed_global_path {
        println!(
            "note: global config exists at {} but is shadowed by the workspace config above \
             (replace-not-merge: only one file is ever loaded)",
            redact_home_dir(&global_path.display().to_string())
        );
    }
    println!();

    println!("settings:");
    for (key, value) in settings {
        println!("  {key:<36} {}", render_value(value));
    }
    println!();

    println!("derived:");
    println!(
        "  {:<36} {}",
        "execution_env_inherit", snapshot.execution_env_inherit
    );
    println!(
        "  {:<36} {}",
        "global_root",
        runtime.global_root().to_string_lossy()
    );
    println!(
        "  {:<36} {}",
        "shared_root",
        runtime.shared_root().to_string_lossy()
    );
    println!(
        "  {:<36} {}",
        "local_root",
        runtime.local_root().to_string_lossy()
    );
    println!("  {:<36} {}", "config_path", store.path().to_string_lossy());
    println!(
        "  {:<36} {}",
        "persistence",
        runtime.persistence_config_json()
    );
}

fn render_value(value: &JsonValue) -> String {
    match value {
        JsonValue::String(s) => format!("\"{s}\""),
        other => other.to_string(),
    }
}
