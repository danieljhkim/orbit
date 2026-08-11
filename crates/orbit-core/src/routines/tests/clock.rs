use std::fs;
use std::path::Path;
use std::sync::Mutex;

use tempfile::tempdir;

use orbit_common::types::OrbitError;

use super::super::clock::{
    ClockCommandRunner, ClockPlatform, ClockSettings, ManagerCommand, clock_status_with,
    load_clock_settings, render_systemd_service, render_systemd_timer, set_clock_cadence_with,
    set_clock_enabled_with,
};

struct MockRunner {
    results: Mutex<Vec<Result<bool, OrbitError>>>,
    commands: Mutex<Vec<String>>,
}

impl MockRunner {
    fn new(results: Vec<Result<bool, OrbitError>>) -> Self {
        Self {
            results: Mutex::new(results.into_iter().rev().collect()),
            commands: Mutex::new(Vec::new()),
        }
    }

    fn commands(&self) -> Vec<String> {
        self.commands.lock().expect("test command log lock").clone()
    }
}

impl ClockCommandRunner for MockRunner {
    fn run(&self, command: &ManagerCommand) -> Result<bool, OrbitError> {
        self.commands
            .lock()
            .expect("test command log lock")
            .push(command.display());
        self.results
            .lock()
            .expect("test result queue lock")
            .pop()
            .expect("test configured a result for every manager command")
    }
}

fn service_path(rendered: &str) -> &str {
    rendered
        .lines()
        .find_map(|line| line.strip_prefix("Environment=PATH="))
        .expect("systemd service declares PATH")
}

fn finds_launcher(path: &str, launcher: &str) -> bool {
    path.split(':')
        .map(Path::new)
        .any(|directory| directory.join(launcher).is_file())
}

#[test]
fn rendered_systemd_service_discovers_local_provider_launchers() {
    let home = tempdir().expect("create temporary home");
    let provider_bin = home.path().join(".local/bin");
    fs::create_dir_all(&provider_bin).expect("create provider directory");
    fs::write(provider_bin.join("codex"), "provider launcher").expect("write provider launcher");

    let manager_path = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
    assert!(
        !finds_launcher(manager_path, "codex"),
        "the minimal user-manager PATH does not find the provider launcher"
    );

    let rendered = render_systemd_service("/opt/orbit/bin/orbit");
    let rendered_path = service_path(&rendered);
    let effective_path = rendered_path.replace("%h", &home.path().to_string_lossy());

    assert!(
        finds_launcher(&effective_path, "codex"),
        "the service PATH finds a provider launcher from the user's .local/bin"
    );
    assert!(rendered_path.starts_with("%h/.local/bin:%h/.orbit/bin:%h/.cargo/bin:"));
    for directory in [
        "/usr/local/sbin",
        "/usr/local/bin",
        "/usr/sbin",
        "/usr/bin",
        "/sbin",
        "/bin",
    ] {
        assert!(rendered_path.split(':').any(|entry| entry == directory));
    }
    assert!(!rendered_path.contains('~'));
    assert!(!rendered_path.contains("/home/"));
    assert!(!rendered_path.contains("/nix/store/"));
    assert!(rendered.contains("Type=oneshot"));
    assert!(rendered.contains("KillMode=process"));
    assert!(rendered.contains("ExecStart=/opt/orbit/bin/orbit sweep"));
}

#[test]
fn systemd_timer_renders_default_and_configured_cadence() {
    assert!(render_systemd_timer(ClockSettings::default()).contains("OnUnitActiveSec=60s"));
    assert!(
        render_systemd_timer(ClockSettings {
            cadence_seconds: 300
        })
        .contains("OnUnitActiveSec=300s")
    );
}

#[test]
fn clock_cadence_rejects_subminute_and_out_of_range_values() {
    assert!(
        ClockSettings {
            cadence_seconds: 30
        }
        .validate()
        .is_err()
    );
    assert!(
        ClockSettings {
            cadence_seconds: 90
        }
        .validate()
        .is_err()
    );
    assert!(
        ClockSettings {
            cadence_seconds: 3_660
        }
        .validate()
        .is_err()
    );
}

#[test]
fn status_is_deterministic_for_each_manager_and_reports_configured_cadence() {
    let root = tempdir().expect("create global root");
    for (platform, manager) in [
        (ClockPlatform::Launchd, "launchd"),
        (ClockPlatform::Systemd, "systemd"),
    ] {
        let runner = MockRunner::new(vec![Ok(true)]);
        let status = clock_status_with(root.path(), platform, &runner).expect("read status");
        assert!(status.enabled);
        assert_eq!(status.configured_cadence_seconds, 60);
        assert_eq!(status.effective_cadence_seconds, Some(60));
        assert_eq!(status.platform, manager);
    }
}

#[test]
fn pause_and_enable_are_idempotent_for_launchd_and_systemd() {
    let root = tempdir().expect("create global root");
    let home = tempdir().expect("create home");
    for (platform, pause, enable) in [
        (ClockPlatform::Launchd, "launchctl unload", "launchctl load"),
        (
            ClockPlatform::Systemd,
            "systemctl --user disable --now orbit-sweep.timer",
            "systemctl --user enable --now orbit-sweep.timer",
        ),
    ] {
        let runner = MockRunner::new(vec![
            Ok(true),
            Ok(true),
            Ok(false),
            Ok(false),
            Ok(true),
            Ok(true),
        ]);
        assert!(
            !set_clock_enabled_with(root.path(), false, platform, &runner, home.path())
                .expect("pause")
                .enabled
        );
        assert!(
            !set_clock_enabled_with(root.path(), false, platform, &runner, home.path())
                .expect("repeat pause")
                .enabled
        );
        assert!(
            set_clock_enabled_with(root.path(), true, platform, &runner, home.path())
                .expect("enable")
                .enabled
        );
        assert!(
            set_clock_enabled_with(root.path(), true, platform, &runner, home.path())
                .expect("repeat enable")
                .enabled
        );
        let commands = runner.commands();
        assert_eq!(
            commands.len(),
            6,
            "only state-changing calls run manager commands"
        );
        assert!(commands[1].starts_with(pause));
        assert!(commands[4].starts_with(enable));
    }
}

#[test]
fn manager_failures_include_exact_recovery_command() {
    let root = tempdir().expect("create global root");
    let home = tempdir().expect("create home");
    let runner = MockRunner::new(vec![Ok(false), Ok(false)]);
    let error = set_clock_enabled_with(
        root.path(),
        true,
        ClockPlatform::Systemd,
        &runner,
        home.path(),
    )
    .expect_err("failed enable is surfaced");
    assert!(
        error
            .to_string()
            .contains("systemctl --user enable --now orbit-sweep.timer")
    );
    assert!(error.to_string().contains("recovery:"));
}

#[test]
fn cadence_reload_failure_restores_config_and_reactivates_previous_systemd_unit() {
    let root = tempdir().expect("create global root");
    let home = tempdir().expect("create home");
    let runner = MockRunner::new(vec![Ok(true), Ok(false), Ok(true), Ok(true)]);
    let error = set_clock_cadence_with(
        root.path(),
        300,
        "/opt/orbit/bin/orbit",
        ClockPlatform::Systemd,
        &runner,
        home.path(),
    )
    .expect_err("failed activation rolls back");
    assert!(
        error
            .to_string()
            .contains("restored the previous configured cadence")
    );
    assert_eq!(
        load_clock_settings(root.path())
            .expect("load restored config")
            .cadence_seconds,
        60
    );
    let timer = fs::read_to_string(home.path().join(".config/systemd/user/orbit-sweep.timer"))
        .expect("read restored timer");
    assert!(timer.contains("OnUnitActiveSec=60s"));
    assert_eq!(
        runner.commands(),
        vec![
            "systemctl --user daemon-reload",
            "systemctl --user enable --now orbit-sweep.timer",
            "systemctl --user daemon-reload",
            "systemctl --user enable --now orbit-sweep.timer",
        ]
    );
}

#[test]
fn launchd_activation_failure_restores_the_previous_rendered_unit() {
    let root = tempdir().expect("create global root");
    let home = tempdir().expect("create home");
    let runner = MockRunner::new(vec![Ok(true), Ok(false), Ok(true), Ok(true)]);
    set_clock_cadence_with(
        root.path(),
        300,
        "/opt/orbit/bin/orbit",
        ClockPlatform::Launchd,
        &runner,
        home.path(),
    )
    .expect_err("failed load rolls back");
    let plist = fs::read_to_string(
        home.path()
            .join("Library/LaunchAgents/com.orbit.sweep.plist"),
    )
    .expect("read restored plist");
    assert!(plist.contains("<integer>60</integer>"));
    assert_eq!(
        runner.commands(),
        vec![
            format!(
                "launchctl unload {}/Library/LaunchAgents/com.orbit.sweep.plist",
                home.path().display()
            ),
            format!(
                "launchctl load {}/Library/LaunchAgents/com.orbit.sweep.plist",
                home.path().display()
            ),
            format!(
                "launchctl unload {}/Library/LaunchAgents/com.orbit.sweep.plist",
                home.path().display()
            ),
            format!(
                "launchctl load {}/Library/LaunchAgents/com.orbit.sweep.plist",
                home.path().display()
            ),
        ]
    );
}
