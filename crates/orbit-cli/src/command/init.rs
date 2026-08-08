use clap::Args;
use orbit_core::command::init::{InitOptions, init_global};
use orbit_core::config::RawAgentRoleConfig;
use orbit_core::config::agent_detect::{DetectedAgents, RealAgentEnvProbe, detect};
use orbit_core::config::agent_prompt::{
    StdinPrompter, collect_qa_crew_setting, collect_role_settings,
};
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_remote::workspace_registry::global_orbit_dir;
use orbit_remote::{
    HostIdentityOutcome, HostMode, NewHostIdentity, ensure_host_identity, os_hostname,
};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::command::{CommandOut, CommandOutput, Execute};

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

    /// Operator-chosen host name for this machine's identity. Used only when
    /// no identity exists yet (first init). Required with --non-interactive on
    /// a fresh host; interactively, the OS hostname is the default.
    #[arg(long)]
    pub host_name: Option<String>,

    /// Host operating mode for this machine's identity: standalone | hub |
    /// spoke. Defaults to standalone. Used only on first init.
    #[arg(long, value_name = "MODE")]
    pub host_mode: Option<String>,
}

impl Execute for InitCommand {
    fn execute(self, _runtime: &OrbitRuntime) -> CommandOut {
        {
            self.run(None)?;
            Ok(CommandOutput::Silent)
        }
    }
}

impl InitCommand {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> CommandOut {
        {
            self.run(root_override)?;
            Ok(CommandOutput::Silent)
        }
    }

    fn run(self, root_override: Option<&Path>) -> Result<(), OrbitError> {
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
        // Host identity is created/migrated here — `orbit init` is its sole
        // owner (ADR-0227). This runs after the root exists so the file has a
        // parent directory.
        ensure_host_identity_for_init(
            root_override,
            self.non_interactive,
            self.host_name,
            self.host_mode,
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

/// Create or migrate this machine's host identity. Host name and mode are only
/// consulted when the identity is absent (a fresh create); a present identity
/// is preserved unchanged and a legacy file is migrated without prompting.
fn ensure_host_identity_for_init(
    root_override: Option<&Path>,
    non_interactive: bool,
    host_name: Option<String>,
    host_mode: Option<String>,
) -> Result<(), OrbitError> {
    let global_root = match root_override {
        Some(root) => root.to_path_buf(),
        None => global_orbit_dir()?,
    };
    // Validate an explicit mode up front so an invalid value fails closed even
    // when the identity already exists.
    let requested_mode = match host_mode.as_deref() {
        Some(mode) => Some(HostMode::parse(mode)?),
        None => None,
    };

    let outcome = ensure_host_identity(&global_root, move || {
        let host_id = match host_name {
            Some(name) => name,
            None if non_interactive => {
                return Err(OrbitError::InvalidInput(
                    "host identity is absent; pass --host-name (and optionally --host-mode) \
                     to initialize a fresh host non-interactively"
                        .to_string(),
                ));
            }
            None => prompt_host_name()?,
        };
        let mode = match requested_mode {
            Some(mode) => mode,
            None if non_interactive => HostMode::Standalone,
            None => prompt_host_mode()?,
        };
        Ok(NewHostIdentity { host_id, mode })
    })?;

    report_host_identity(&outcome);
    Ok(())
}

fn report_host_identity(outcome: &HostIdentityOutcome) {
    let identity = outcome.identity();
    let verb = match outcome {
        HostIdentityOutcome::Created(_) => "created",
        HostIdentityOutcome::Migrated(_) => "migrated",
        HostIdentityOutcome::Unchanged(_) => "unchanged",
    };
    println!(
        "host identity ({verb}): host_id=\"{}\", machine_id={}, mode={}",
        identity.host_id, identity.machine_id, identity.mode
    );
}

fn prompt_host_name() -> Result<String, OrbitError> {
    let default = os_hostname();
    let prompt = match default.as_deref() {
        Some(name) => format!("Host name [{name}]: "),
        None => "Host name: ".to_string(),
    };
    let answer = read_line(&prompt)?;
    if answer.is_empty() {
        default.ok_or_else(|| {
            OrbitError::InvalidInput(
                "no host name entered and the OS hostname is unavailable; \
                 re-run with --host-name"
                    .to_string(),
            )
        })
    } else {
        Ok(answer)
    }
}

fn prompt_host_mode() -> Result<HostMode, OrbitError> {
    loop {
        let answer = read_line("Host mode (standalone/hub/spoke) [standalone]: ")?;
        if answer.is_empty() {
            return Ok(HostMode::Standalone);
        }
        match HostMode::parse(&answer) {
            Ok(mode) => return Ok(mode),
            Err(error) => println!("{error}"),
        }
    }
}

fn read_line(prompt: &str) -> Result<String, OrbitError> {
    let mut stdout = io::stdout();
    write!(stdout, "{prompt}").map_err(|error| OrbitError::Io(error.to_string()))?;
    stdout
        .flush()
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    Ok(line.trim().to_string())
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
    let mut collected = collect_role_settings(detected, &mut prompter)
        .map_err(|err| OrbitError::Io(format!("agent prompts failed: {err}")))?;
    let qa = collect_qa_crew_setting(detected, &mut prompter)
        .map_err(|err| OrbitError::Io(format!("QA crew prompt failed: {err}")))?;
    if let Some(qa) = qa {
        collected.insert("qa".to_string(), qa);
    }
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
    if root_override.is_some_and(|path| !orbit_core::runtime::is_global_orbit_root(path)) {
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
