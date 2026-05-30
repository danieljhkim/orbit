use std::fs;
use std::sync::Mutex;
use tempfile::tempdir;

use crate::InitCommand;
use crate::command::init::collect_role_settings_for_init;
use orbit_core::config::agent_detect::DetectedAgents;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// `collect_role_settings_for_init` short-circuits when --non-interactive
/// is set, regardless of whether config.toml exists. No prompts are
/// attempted (we can't stub stdin from here, so the test passing without
/// hanging is the proof).
#[test]
fn non_interactive_short_circuits_before_prompts() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let home = tempdir().expect("home tempdir");
    let detected = DetectedAgents::default();
    let result = collect_role_settings_for_init(Some(home.path()), false, true, &detected);
    assert!(matches!(result, Ok(None)));
}

/// When config.toml already exists and --force is unset, prompts are
/// skipped — `orbit init` is idempotent over an existing global root.
#[test]
fn existing_config_short_circuits_before_prompts() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let root = tempdir().expect("orbit root");
    let config_path = root.path().join("config.toml");
    fs::write(&config_path, "# pre-existing\n").expect("preseed");

    let detected = DetectedAgents::default();
    let result = collect_role_settings_for_init(Some(root.path()), false, false, &detected);
    assert!(matches!(result, Ok(None)));
}

/// End-to-end: `InitCommand { non_interactive: true }` produces a fresh
/// config.toml with generated crew tables and no uncommented `[agent.*]`
/// sections. The exact default crew/duel set depends on the runner PATH.
#[test]
fn non_interactive_init_writes_generated_crew_config() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let home = tempdir().expect("home tempdir");

    let cmd = InitCommand {
        force: false,
        non_interactive: true,
    };
    let outcome = cmd.execute_without_runtime(Some(&home.path().join(".orbit")));

    outcome.expect("init succeeded");

    let config_path = home.path().join(".orbit").join("config.toml");
    let contents = fs::read_to_string(&config_path).expect("read config");
    for line in contents.lines() {
        assert!(
            !line.trim_start().starts_with("[agent."),
            "unexpected uncommented agent section: {line}",
        );
    }
    assert!(contents.contains("[crews.claude]"));
    assert!(contents.contains("[crews.codex]"));
    assert!(contents.contains("[crews.gemini]"));
    assert!(contents.contains("[crews.grok]"));
    toml::from_str::<toml::Value>(&contents).expect("seeded config parses");
}
