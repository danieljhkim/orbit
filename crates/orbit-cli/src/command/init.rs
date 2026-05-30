use clap::Args;
use orbit_core::command::init::{InitOptions, init_global};
use orbit_core::config::RawAgentRoleConfig;
use orbit_core::config::agent_detect::{DetectedAgents, RealAgentEnvProbe, detect};
use orbit_core::config::agent_prompt::{StdinPrompter, collect_role_settings};
use orbit_core::workspace_registry::global_orbit_dir;
use orbit_core::{OrbitError, OrbitRuntime};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Initialize the global Orbit root (~/.orbit)")]
pub struct InitCommand {
    /// Reset the global Orbit root (~/.orbit/) to defaults before initialization
    #[arg(long)]
    pub force: bool,

    /// Skip interactive prompts. config.toml is still seeded from detected
    /// agent surfaces, but a CI runner that pipes nothing into stdin will not
    /// hang.
    #[arg(long)]
    pub non_interactive: bool,
}

impl Execute for InitCommand {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let detected = detect(&RealAgentEnvProbe);
        let role_settings =
            collect_role_settings_for_init(None, self.force, self.non_interactive, &detected)?;
        let result = init_global(
            None,
            InitOptions {
                force: self.force,
                refresh_defaults: true,
                role_settings,
                detected: Some(detected),
                ..Default::default()
            },
        )?;
        let paths = reported_init_paths(None);
        print_init_result(InitOutput {
            skills_root: paths.skills_root,
            refreshed_skill_files: result.refreshed_skill_files,
            created_skills_symlink: result.created_skills_symlink,
            config_path: paths.config_path,
            created_config: result.created_config,
            refreshed_default_activities: result.refreshed_default_activities,
            refreshed_default_jobs: result.refreshed_default_jobs,
            refreshed_default_executors: result.refreshed_default_executors,
            refreshed_default_policies: result.refreshed_default_policies,
        });
        Ok(())
    }
}

impl InitCommand {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> Result<(), OrbitError> {
        let detected = detect(&RealAgentEnvProbe);
        let role_settings = collect_role_settings_for_init(
            root_override,
            self.force,
            self.non_interactive,
            &detected,
        )?;
        let result = init_global(
            root_override,
            InitOptions {
                force: self.force,
                refresh_defaults: true,
                role_settings,
                detected: Some(detected),
                ..Default::default()
            },
        )?;
        let paths = reported_init_paths(root_override);
        print_init_result(InitOutput {
            skills_root: paths.skills_root,
            refreshed_skill_files: result.refreshed_skill_files,
            created_skills_symlink: result.created_skills_symlink,
            config_path: paths.config_path,
            created_config: result.created_config,
            refreshed_default_activities: result.refreshed_default_activities,
            refreshed_default_jobs: result.refreshed_default_jobs,
            refreshed_default_executors: result.refreshed_default_executors,
            refreshed_default_policies: result.refreshed_default_policies,
        });
        Ok(())
    }
}

/// Decide whether to prompt for `[agent.<role>]` settings and collect them.
///
/// Prompts run only when ALL of:
/// - `--non-interactive` is unset
/// - the target config.toml does not already exist (or `--force` is set, which
///   wipes it)
pub(crate) fn collect_role_settings_for_init(
    root_override: Option<&Path>,
    force: bool,
    non_interactive: bool,
    detected: &DetectedAgents,
) -> Result<Option<BTreeMap<String, RawAgentRoleConfig>>, OrbitError> {
    if non_interactive {
        return Ok(None);
    }

    let config_path = resolve_config_path(root_override)?;
    if config_path.exists() && !force {
        return Ok(None);
    }

    let mut prompter = StdinPrompter;
    let collected = collect_role_settings(detected, &mut prompter)
        .map_err(|err| OrbitError::Io(format!("agent prompts failed: {err}")))?;
    Ok(Some(collected))
}

fn resolve_config_path(root_override: Option<&Path>) -> Result<PathBuf, OrbitError> {
    let root = match root_override {
        Some(root) => root.to_path_buf(),
        None => global_orbit_dir()?,
    };
    Ok(root.join("config.toml"))
}

fn print_init_result(output: InitOutput) {
    println!(
        "skills: root={}, refreshed={}, symlink_created={}; config: path={}, created={}; default_activities_refreshed={}; default_jobs_refreshed={}; default_executors_refreshed={}; default_policies_refreshed={}",
        output.skills_root,
        output.refreshed_skill_files,
        output.created_skills_symlink,
        output.config_path,
        output.created_config,
        output.refreshed_default_activities,
        output.refreshed_default_jobs,
        output.refreshed_default_executors,
        output.refreshed_default_policies,
    );
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct InitOutput {
    skills_root: &'static str,
    refreshed_skill_files: usize,
    created_skills_symlink: bool,
    config_path: &'static str,
    created_config: bool,
    refreshed_default_activities: usize,
    refreshed_default_jobs: usize,
    refreshed_default_executors: usize,
    refreshed_default_policies: usize,
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
