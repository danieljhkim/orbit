use std::fs;

use tempfile::tempdir;

use crate::command::init::agent_detect::{DetectedAgents, detect, testing::MockAgentEnvProbe};
use crate::command::init::seed::{collect_crew_settings_for_init, config_seed_from_detection};
use crate::tests::env_isolation::EnvGuard;

/// `collect_crew_settings_for_init` short-circuits when --non-interactive
/// is set, regardless of whether config.toml exists. No prompts are
/// attempted (we can't stub stdin from here, so the test passing without
/// hanging is the proof).
#[test]
fn non_interactive_short_circuits_before_prompts() {
    let _env = EnvGuard::acquire();
    let home = tempdir().expect("home tempdir");
    let detected = DetectedAgents::default();
    let result = collect_crew_settings_for_init(Some(home.path()), false, true, &detected);
    assert!(matches!(result, Ok(None)));
}

/// When config.toml already exists and --force is unset, prompts are
/// skipped — `orbit init` is idempotent over an existing global root.
#[test]
fn existing_config_short_circuits_before_prompts() {
    let _env = EnvGuard::acquire();
    let root = tempdir().expect("orbit root");
    let config_path = root.path().join("config.toml");
    fs::write(&config_path, "# pre-existing\n").expect("preseed");

    let detected = DetectedAgents::default();
    let result = collect_crew_settings_for_init(Some(root.path()), false, false, &detected);
    assert!(matches!(result, Ok(None)));
}

/// The seed carries only families Orbit ships crews for. `ollama` is detected
/// for the prompt's benefit but must not reach config seeding, which has no
/// ollama crew to write.
#[test]
fn seed_projects_detected_clis_onto_crew_families_only() {
    let detected = detect(
        &MockAgentEnvProbe::new()
            .with_binary("claude")
            .with_binary("grok")
            .with_binary("ollama"),
    );

    let seed = config_seed_from_detection(&detected);

    assert_eq!(
        seed.families.iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["claude", "grok"]
    );
    assert!(seed.crews.is_empty());
}

#[test]
fn seed_is_empty_when_no_provider_cli_is_installed() {
    let seed = config_seed_from_detection(&DetectedAgents::default());

    assert!(seed.families.is_empty());
}
