use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tempfile::tempdir;

use crate::InitCommand;
use crate::command::init::collect_role_settings_for_init;
use orbit_common::utility::fs::create_dir_symlink;
use orbit_core::config::agent_detect::DetectedAgents;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct ScopedHome {
    previous_home: Option<std::ffi::OsString>,
    previous_userprofile: Option<std::ffi::OsString>,
}

impl ScopedHome {
    fn set(path: &Path) -> Self {
        let guard = Self {
            previous_home: std::env::var_os("HOME"),
            previous_userprofile: std::env::var_os("USERPROFILE"),
        };
        unsafe {
            std::env::set_var("HOME", path);
            std::env::set_var("USERPROFILE", path);
        }
        guard
    }
}

impl Drop for ScopedHome {
    fn drop(&mut self) {
        restore_env("HOME", self.previous_home.take());
        restore_env("USERPROFILE", self.previous_userprofile.take());
    }
}

fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
    match value {
        Some(value) => unsafe { std::env::set_var(name, value) },
        None => unsafe { std::env::remove_var(name) },
    }
}

fn seed_discovery_sentinel(home: &Path, agent_dir: &str) -> (PathBuf, PathBuf) {
    let target = home.join("live-sentinels").join(agent_dir).join("orbit");
    fs::create_dir_all(&target).expect("create sentinel target");
    let link = home.join(agent_dir).join("skills").join("orbit");
    fs::create_dir_all(link.parent().expect("sentinel link parent"))
        .expect("create sentinel link parent");
    create_dir_symlink(&target, &link).expect("create sentinel discovery link");
    (link, target)
}

fn assert_discovery_sentinel(link: &Path, target: &Path) {
    assert_eq!(
        fs::read_link(link).expect("read sentinel discovery link"),
        target,
        "temporary-root validation must not rewrite live skill discovery links",
    );
}

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

/// End-to-end: temporary-root validation runs with an isolated discovery
/// home, preserving the invoking account's existing skill links while it
/// produces a fresh config.toml with generated crew tables.
#[test]
fn non_interactive_init_isolates_temporary_root_skill_discovery() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let live_home = tempdir().expect("live home tempdir");
    let validation_home = tempdir().expect("validation home tempdir");
    let validation_root = tempdir().expect("validation root tempdir");

    let _live_home = ScopedHome::set(live_home.path());
    let (agents_link, agents_target) = seed_discovery_sentinel(live_home.path(), ".agents");
    let (claude_link, claude_target) = seed_discovery_sentinel(live_home.path(), ".claude");

    let outcome = {
        let _validation_home = ScopedHome::set(validation_home.path());
        InitCommand {
            force: false,
            non_interactive: true,
        }
        .execute_without_runtime(Some(&validation_root.path().join(".orbit")))
    };

    outcome.expect("init succeeded");

    assert_discovery_sentinel(&agents_link, &agents_target);
    assert_discovery_sentinel(&claude_link, &claude_target);
    assert!(
        validation_home
            .path()
            .join(".agents")
            .join("skills")
            .join("orbit")
            .is_symlink(),
        "validation links belong under the isolated HOME"
    );
    assert!(
        validation_home
            .path()
            .join(".claude")
            .join("skills")
            .join("orbit")
            .is_symlink(),
        "validation links belong under the isolated HOME"
    );

    let config_path = validation_root.path().join(".orbit").join("config.toml");
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
    assert!(contents.contains("[crews.qa]"));
    let config = toml::from_str::<toml::Value>(&contents).expect("seeded config parses");
    let codex = config
        .get("crews")
        .and_then(|crews| crews.get("codex"))
        .expect("codex crew is seeded");
    assert_eq!(
        codex.get("model").and_then(toml::Value::as_str),
        Some("gpt-5.6-terra"),
    );
    drop(validation_root);
    drop(validation_home);
    assert_discovery_sentinel(&agents_link, &agents_target);
    assert_discovery_sentinel(&claude_link, &claude_target);
}
