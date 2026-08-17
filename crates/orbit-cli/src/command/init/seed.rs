//! Turn host detection and interactive answers into an `orbit_config::ConfigSeed`.
//!
//! This is the whole adapter between the terminal/host and the config crate:
//! detection and prompting happen here, and everything that crosses into
//! `orbit-config` is plain data.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use orbit_config::{ConfigSeed, CrewSeed};
use orbit_core::OrbitError;
use orbit_registry::workspace_registry::global_orbit_dir;

use super::agent_detect::{DetectedAgents, RealAgentEnvProbe, available_crew_families, detect};
use super::agent_prompt::{
    Prompter, StdinPrompter, collect_crew_setting, collect_system_crew_setting,
};

/// Probe the host and build the seed for `orbit init`.
///
/// Prompts run only when ALL of:
/// - `--non-interactive` is unset
/// - the target config.toml does not already exist (or `--force` is set, which
///   wipes it)
///
/// A non-interactive run still seeds from detected agent surfaces; only the
/// crew prompts are skipped.
pub(crate) fn collect_config_seed_for_init(
    root_override: Option<&Path>,
    force: bool,
    non_interactive: bool,
) -> Result<ConfigSeed, OrbitError> {
    let detected = detect(&RealAgentEnvProbe);
    let seed = config_seed_from_detection(&detected);
    let crews = collect_crew_settings_for_init(root_override, force, non_interactive, &detected)?;
    Ok(match crews {
        Some(crews) => seed.with_crews(crews),
        None => seed,
    })
}

/// The host-blind projection of a detection snapshot: which crew families this
/// machine can actually dispatch to.
pub(crate) fn config_seed_from_detection(detected: &DetectedAgents) -> ConfigSeed {
    ConfigSeed::from_families(available_crew_families(detected))
}

/// Decide whether to prompt for the default and system crew settings, and
/// collect them when so. QA is never prompted: leftover `[crews.qa]` stays a
/// silently auto-seeded compatibility crew.
pub(crate) fn collect_crew_settings_for_init(
    root_override: Option<&Path>,
    force: bool,
    non_interactive: bool,
    detected: &DetectedAgents,
) -> Result<Option<BTreeMap<String, CrewSeed>>, OrbitError> {
    if non_interactive {
        return Ok(None);
    }

    let config_path = resolve_config_path(root_override)?;
    if config_path.exists() && !force {
        return Ok(None);
    }

    let mut prompter = StdinPrompter;
    collect_interactive_crew_settings(detected, &mut prompter)
        .map(Some)
        .map_err(|err| OrbitError::Io(format!("agent prompts failed: {err}")))
}

/// Prompt for `[crews.custom]` and, when more than one cheap-tier family is
/// detected, `[crews.system]`. Does not prompt for QA.
pub(crate) fn collect_interactive_crew_settings(
    detected: &DetectedAgents,
    prompter: &mut dyn Prompter,
) -> io::Result<BTreeMap<String, CrewSeed>> {
    let custom = collect_crew_setting(detected, prompter)?;
    let mut collected = BTreeMap::from([("custom".to_string(), custom)]);
    if let Some(system) = collect_system_crew_setting(detected, prompter)? {
        collected.insert("system".to_string(), system);
    }
    Ok(collected)
}

fn resolve_config_path(root_override: Option<&Path>) -> Result<PathBuf, OrbitError> {
    let root = match root_override {
        Some(root) => root.to_path_buf(),
        None => global_orbit_dir()?,
    };
    Ok(root.join("config.toml"))
}
