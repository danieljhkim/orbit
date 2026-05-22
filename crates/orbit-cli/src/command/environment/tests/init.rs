use std::fs;
use std::sync::Mutex;
use tempfile::tempdir;

use crate::InitCommand;
use crate::command::init::collect_role_settings_for_init;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn restore_home(previous_home: Option<std::ffi::OsString>) {
    match previous_home {
        Some(value) => unsafe {
            std::env::set_var("HOME", value);
        },
        None => unsafe {
            std::env::remove_var("HOME");
        },
    }
}

/// `collect_role_settings_for_init` short-circuits when --non-interactive
/// is set, regardless of whether config.toml exists. No prompts are
/// attempted (we can't stub stdin from here, so the test passing without
/// hanging is the proof).
#[test]
fn non_interactive_short_circuits_before_prompts() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let home = tempdir().expect("home tempdir");
    let result = collect_role_settings_for_init(Some(home.path()), false, true);
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

    let result = collect_role_settings_for_init(Some(root.path()), false, false);
    assert!(matches!(result, Ok(None)));
}

/// End-to-end: `InitCommand { non_interactive: true }` produces a fresh
/// config.toml that contains no uncommented `[agent.*]` sections.
#[test]
fn non_interactive_init_writes_no_active_agent_sections() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let home = tempdir().expect("home tempdir");
    let previous_home = std::env::var_os("HOME");
    unsafe {
        std::env::set_var("HOME", home.path());
    }

    let cmd = InitCommand {
        force: false,
        non_interactive: true,
    };
    let outcome = cmd.execute_without_runtime(Some(&home.path().join(".orbit")));
    restore_home(previous_home);

    outcome.expect("init succeeded");

    let config_path = home.path().join(".orbit").join("config.toml");
    let contents = fs::read_to_string(&config_path).expect("read config");
    for line in contents.lines() {
        assert!(
            !line.trim_start().starts_with("[agent."),
            "unexpected uncommented agent section: {line}",
        );
    }
}
