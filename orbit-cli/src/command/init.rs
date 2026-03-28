use clap::Args;
use orbit_core::command::init::{InitOptions, init_global};
use orbit_core::{OrbitError, OrbitRuntime};
use std::path::{Path, PathBuf};

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
                ..Default::default()
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
                ..Default::default()
            },
        )?;
        print_init_result(&result);
        Ok(())
    }
}

fn print_init_result(result: &orbit_core::command::init::InitResult) {
    println!(
        "skills: root={}, refreshed={}, symlink_created={}; config: path={}, created={}; default_activities_refreshed={}; default_jobs_refreshed={}",
        init_path_label(&result.skills_root, InitPathKind::SkillsRoot),
        result.refreshed_skill_files,
        result.created_skills_symlink,
        init_path_label(&result.config_path, InitPathKind::ConfigPath),
        result.created_config,
        result.refreshed_default_activities,
        result.refreshed_default_jobs
    );
}

#[derive(Copy, Clone)]
enum InitPathKind {
    SkillsRoot,
    ConfigPath,
}

fn init_path_label(path: &str, kind: InitPathKind) -> &'static str {
    if global_orbit_path_for(kind).is_some_and(|expected| Path::new(path) == expected) {
        match kind {
            InitPathKind::SkillsRoot => "~/.orbit/skills",
            InitPathKind::ConfigPath => "~/.orbit/config.toml",
        }
    } else {
        match kind {
            InitPathKind::SkillsRoot => "<custom orbit root>/skills",
            InitPathKind::ConfigPath => "<custom orbit root>/config.toml",
        }
    }
}

fn global_orbit_path_for(kind: InitPathKind) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let orbit_root = PathBuf::from(home).join(".orbit");
    Some(match kind {
        InitPathKind::SkillsRoot => orbit_root.join("skills"),
        InitPathKind::ConfigPath => orbit_root.join("config.toml"),
    })
}
