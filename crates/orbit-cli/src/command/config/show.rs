use clap::Args;
use orbit_common::utility::redaction::redact_home_dir;
use orbit_core::config::{EffectiveConfigValue, load_effective_config};
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
        if self.scope == ConfigScopeArg::Effective {
            let effective = load_effective_config(&runtime.global_root(), &runtime.shared_root())?;
            return if self.json {
                print_effective_json(runtime, effective.values())
            } else {
                print_effective_text(runtime, effective.values());
                Ok(())
            };
        }

        let store = open_store_for_scope(runtime, self.scope)?;
        let snapshot = store.snapshot()?;
        let settings = snapshot.all_values();

        if self.json {
            print_json(runtime, &store, &snapshot, &settings)
        } else {
            print_text(runtime, &store, &snapshot, &settings);
            Ok(())
        }
    }
}

fn print_effective_json(
    runtime: &OrbitRuntime,
    values: &[EffectiveConfigValue],
) -> Result<(), OrbitError> {
    crate::output::json::print_pretty(&effective_json(runtime, values))
}

pub(super) fn effective_json(runtime: &OrbitRuntime, values: &[EffectiveConfigValue]) -> JsonValue {
    let mut settings = Map::new();
    let mut provenance = Map::new();
    for entry in values {
        settings.insert(entry.key.clone(), entry.value.clone());
        provenance.insert(
            entry.key.clone(),
            json!({
                "scope": entry.source.kind().label(),
                "path": entry.source.path().map(|path| path.to_string_lossy().into_owned()),
            }),
        );
    }
    let global_path = global_config_path(runtime);
    let workspace_path = runtime.shared_root().join("config.toml");
    let config_path = if workspace_path.exists() && runtime.shared_root() != runtime.global_root() {
        &workspace_path
    } else {
        &global_path
    };

    json!({
        "source": {
            "scope": "effective",
            "global_path": global_path.to_string_lossy(),
            "workspace_path": workspace_path.to_string_lossy(),
        },
        "shadowed_global_path": JsonValue::Null,
        "settings": settings,
        "provenance": provenance,
        "execution_env_inherit": false,
        "global_root": runtime.global_root().to_string_lossy(),
        "shared_root": runtime.shared_root().to_string_lossy(),
        "local_root": runtime.local_root().to_string_lossy(),
        "config_path": config_path.to_string_lossy(),
        "persistence": runtime.persistence_config_json(),
    })
}

fn print_effective_text(runtime: &OrbitRuntime, values: &[EffectiveConfigValue]) {
    println!("source: effective layered configuration");
    println!(
        "  global:    {}",
        redact_home_dir(&global_config_path(runtime).display().to_string())
    );
    println!(
        "  workspace: {}",
        redact_home_dir(
            &runtime
                .shared_root()
                .join("config.toml")
                .display()
                .to_string()
        )
    );
    println!();

    println!("settings:");
    for entry in values {
        let source = match entry.source.path() {
            Some(path) => format!(
                "{} ({})",
                entry.source.kind().label(),
                redact_home_dir(&path.display().to_string())
            ),
            None => entry.source.kind().label().to_string(),
        };
        println!(
            "  {:<36} {:<24} [{}]",
            entry.key,
            render_value(&entry.value),
            source
        );
    }
    println!();

    println!("derived:");
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
    println!(
        "  {:<36} {}",
        "persistence",
        runtime.persistence_config_json()
    );
}

fn print_json(
    runtime: &OrbitRuntime,
    store: &orbit_core::config::ConfigStore,
    snapshot: &orbit_core::config::ConfigSnapshot,
    settings: &[(&'static str, JsonValue)],
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
        "shadowed_global_path": JsonValue::Null,
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
) {
    println!(
        "source: {} ({})",
        store.scope().label(),
        redact_home_dir(&store.path().display().to_string())
    );
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
