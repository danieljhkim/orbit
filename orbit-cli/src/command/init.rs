use clap::Args;
use orbit_core::command::init::{InitOptions, init_global};
use orbit_core::{OrbitError, OrbitRuntime};
use std::path::Path;

use crate::command::Execute;

#[derive(Args)]
pub struct InitCommand {
    /// Reset the global Orbit root (~/.orbit/) to defaults before initialization
    #[arg(long)]
    pub force: bool,
}

impl Execute for InitCommand {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        // Even with a runtime, orbit init targets the global root
        let result = init_global(
            None,
            InitOptions {
                force: self.force,
                refresh_defaults: true,
            },
        )?;
        print_init_result(&result);
        Ok(())
    }
}

impl InitCommand {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> Result<(), OrbitError> {
        let result = init_global(
            root_override,
            InitOptions {
                force: self.force,
                refresh_defaults: true,
            },
        )?;
        print_init_result(&result);
        Ok(())
    }
}

fn print_init_result(result: &orbit_core::command::init::InitResult) {
    println!(
        "skills: root={}, refreshed={}, symlink_created={}; config: path={}, created={}; default_activities_refreshed={}; default_jobs_refreshed={}",
        result.skills_root,
        result.refreshed_skill_files,
        result.created_skills_symlink,
        result.config_path,
        result.created_config,
        result.refreshed_default_activities,
        result.refreshed_default_jobs
    );
}
