use clap::Args;
use orbit_core::OrbitRuntime;
use orbit_core::config::{ConfigScope, ConfigStore, WorkspaceInitMode};

use crate::command::{CommandOut, CommandOutput, Execute};

use super::support::{global_config_path, workspace_config_path};

#[derive(Args)]
pub struct ConfigSetArgs {
    /// Dotted config.toml key, e.g. workflow.base_branch
    pub key: String,
    /// Value to set, parsed as a TOML literal (bool/int/float/array/etc),
    /// falling back to a plain string when it doesn't parse as one
    pub value: String,
    /// Target the global config (~/.orbit/config.toml) instead of the
    /// workspace one
    #[arg(long, conflicts_with_all = ["seed_from_global", "fresh"])]
    pub global: bool,
    /// When the workspace config.toml doesn't exist yet, seed it from the
    /// current global config before applying this edit
    #[arg(long = "seed-from-global", conflicts_with = "fresh")]
    pub seed_from_global: bool,
    /// When the workspace config.toml doesn't exist yet, start from an
    /// empty document before applying this edit
    #[arg(long)]
    pub fresh: bool,
}

impl Execute for ConfigSetArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let mut store = if self.global {
            ConfigStore::open(ConfigScope::Global, global_config_path(runtime))?
        } else {
            let mode = if self.seed_from_global {
                WorkspaceInitMode::SeedFromGlobal
            } else if self.fresh {
                WorkspaceInitMode::Fresh
            } else {
                WorkspaceInitMode::RequireExisting
            };
            ConfigStore::open_for_workspace_set(
                workspace_config_path(runtime),
                &global_config_path(runtime),
                mode,
            )?
        };

        store.set_value(&self.key, &self.value)?;
        store.validate()?;
        store.save()?;

        println!(
            "set {} ({} config: {})",
            self.key,
            store.scope().label(),
            store.path().to_string_lossy()
        );
        Ok(CommandOutput::Silent)
    }
}
