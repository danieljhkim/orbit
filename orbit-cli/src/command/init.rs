use clap::Args;
use orbit_core::command::init::{InitOptions, InitResult, init_global};
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
        print_init_result(&result, reported_init_paths(None));
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
        print_init_result(&result, reported_init_paths(root_override));
        Ok(())
    }
}

fn print_init_result(result: &InitResult, paths: ReportedInitPaths) {
    println!(
        "skills: root={}, refreshed={}, symlink_created={}; config: path={}, created={}; default_activities_refreshed={}; default_jobs_refreshed={}",
        paths.skills_root,
        result.refreshed_skill_files,
        result.created_skills_symlink,
        paths.config_path,
        result.created_config,
        result.refreshed_default_activities,
        result.refreshed_default_jobs
    );
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct ReportedInitPaths {
    skills_root: &'static str,
    config_path: &'static str,
}

fn reported_init_paths(root_override: Option<&Path>) -> ReportedInitPaths {
    if root_override.is_some_and(|path| !is_global_orbit_root(path)) {
        ReportedInitPaths {
            skills_root: "<custom orbit root>/skills",
            config_path: "<custom orbit root>/config.toml",
        }
    } else {
        ReportedInitPaths {
            skills_root: "~/.orbit/skills",
            config_path: "~/.orbit/config.toml",
        }
    }
}

fn is_global_orbit_root(path: &Path) -> bool {
    global_orbit_root().is_some_and(|expected| path == expected)
}

fn global_orbit_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".orbit"))
}

#[cfg(test)]
mod tests {
    use super::{ReportedInitPaths, reported_init_paths};
    use std::path::Path;

    #[test]
    fn reported_init_paths_redact_custom_root() {
        assert_eq!(
            reported_init_paths(Some(Path::new("/tmp/custom/.orbit"))),
            ReportedInitPaths {
                skills_root: "<custom orbit root>/skills",
                config_path: "<custom orbit root>/config.toml",
            }
        );
    }

    #[test]
    fn reported_init_paths_defaults_to_global_root_labels() {
        assert_eq!(
            reported_init_paths(None),
            ReportedInitPaths {
                skills_root: "~/.orbit/skills",
                config_path: "~/.orbit/config.toml",
            }
        );
    }
}
