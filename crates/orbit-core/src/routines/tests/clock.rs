use std::fs;
use std::path::Path;
use std::sync::Mutex;

use tempfile::tempdir;

use orbit_common::OrbitError;

use super::super::clock::{
    ClockCommandRunner, ClockPlatform, ClockSettings, ManagerCommand, clock_status_with,
    install_clock_with, load_clock_settings, render_systemd_service, render_systemd_timer,
    set_clock_cadence_with, set_clock_enabled_with,
};

struct MockRunner {
    results: Mutex<Vec<Result<bool, OrbitError>>>,
    outputs: Mutex<Vec<Result<Option<String>, OrbitError>>>,
    commands: Mutex<Vec<String>>,
}

impl MockRunner {
    fn new(results: Vec<Result<bool, OrbitError>>) -> Self {
        Self {
            results: Mutex::new(results.into_iter().rev().collect()),
            outputs: Mutex::new(Vec::new()),
            commands: Mutex::new(Vec::new()),
        }
    }

    fn with_outputs(
        results: Vec<Result<bool, OrbitError>>,
        outputs: Vec<Result<Option<String>, OrbitError>>,
    ) -> Self {
        Self {
            results: Mutex::new(results.into_iter().rev().collect()),
            outputs: Mutex::new(outputs.into_iter().rev().collect()),
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

    fn stdout(&self, command: &ManagerCommand) -> Result<Option<String>, OrbitError> {
        self.commands
            .lock()
            .expect("test command log lock")
            .push(command.display());
        self.outputs
            .lock()
            .expect("test output queue lock")
            .pop()
            .expect("test configured output for every manager query")
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
    let default_timer = render_systemd_timer(ClockSettings::default());
    assert!(default_timer.contains("OnStartupSec=60s"));
    assert!(default_timer.contains("OnUnitActiveSec=60s"));
    assert!(!default_timer.contains("Persistent=true"));

    let configured_timer = render_systemd_timer(ClockSettings {
        cadence_seconds: 300,
    });
    assert!(configured_timer.contains("OnStartupSec=300s"));
    assert!(configured_timer.contains("OnUnitActiveSec=300s"));
}

#[test]
fn systemd_install_arms_a_fresh_manager_and_recurs_at_configured_cadence() {
    let root = tempdir().expect("create global root");
    let home = tempdir().expect("create home");
    let runner = MockRunner::new(vec![Ok(true), Ok(true), Ok(true)]);

    let report = install_clock_with(
        root.path(),
        "/opt/orbit/bin/orbit",
        ClockSettings {
            cadence_seconds: 300,
        },
        ClockPlatform::Systemd,
        &runner,
        home.path(),
    )
    .expect("install systemd clock");

    assert!(report.activated);
    assert_eq!(
        runner.commands(),
        vec![
            "systemctl --user daemon-reload",
            "systemctl --user enable orbit-sweep.timer",
            "systemctl --user restart orbit-sweep.timer",
        ]
    );
    let timer = fs::read_to_string(home.path().join(".config/systemd/user/orbit-sweep.timer"))
        .expect("read installed timer");
    assert!(timer.contains("OnStartupSec=300s"));
    assert!(timer.contains("OnUnitActiveSec=300s"));
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
    let launchd = MockRunner::new(vec![Ok(true)]);
    let status = clock_status_with(root.path(), ClockPlatform::Launchd, &launchd)
        .expect("read launchd status");
    assert!(status.enabled);
    assert!(status.schedulable);
    assert_eq!(status.configured_cadence_seconds, 60);
    assert_eq!(status.effective_cadence_seconds, Some(60));
    assert_eq!(status.platform, "launchd");

    let systemd = MockRunner::with_outputs(
        vec![Ok(true)],
        vec![Ok(Some(
            "LoadState=loaded\nActiveState=active\nNextElapseUSecRealtime=Sun 2026-08-16 04:30:00 UTC\nNextElapseUSecMonotonic=5min\nLastTriggerUSec=Sun 2026-08-16 04:29:00 UTC".to_string(),
        ))],
    );
    let status = clock_status_with(root.path(), ClockPlatform::Systemd, &systemd)
        .expect("read systemd status");
    assert!(status.enabled);
    assert!(status.schedulable);
    assert_eq!(status.effective_cadence_seconds, Some(60));
    assert!(status.health_issue.is_none());
    assert!(status.loaded);
    assert_eq!(status.running, Some(true));
    assert_eq!(
        status.last_tick_at.as_deref(),
        Some("Sun 2026-08-16 04:29:00 UTC")
    );
    assert_eq!(
        status.next_tick_at.as_deref(),
        Some("Sun 2026-08-16 04:30:00 UTC")
    );
    assert_eq!(status.platform, "systemd");
}

#[test]
fn enabled_systemd_timer_without_a_future_trigger_is_unhealthy() {
    let root = tempdir().expect("create global root");
    let runner = MockRunner::with_outputs(
        vec![Ok(true)],
        vec![Ok(Some(
            "NextElapseUSecRealtime=\nNextElapseUSecMonotonic=0".to_string(),
        ))],
    );

    let status = clock_status_with(root.path(), ClockPlatform::Systemd, &runner)
        .expect("read elapsed timer status");

    assert!(status.enabled);
    assert!(!status.schedulable);
    assert_eq!(status.effective_cadence_seconds, None);
    assert!(status.health_issue.as_deref().is_some_and(|issue| {
        issue.contains("no future trigger") && issue.contains("orbit routine init --install-clock")
    }));
    assert_eq!(
        runner.commands(),
        vec![
            "systemctl --user is-enabled orbit-sweep.timer",
            "systemctl --user show orbit-sweep.timer --property=LoadState --property=ActiveState --property=NextElapseUSecRealtime --property=NextElapseUSecMonotonic --property=LastTriggerUSec",
        ]
    );
}

#[test]
fn disabled_systemd_timer_reports_loaded_state_without_becoming_schedulable() {
    let root = tempdir().expect("create global root");
    let runner = MockRunner::with_outputs(
        vec![Ok(false)],
        vec![Ok(Some(
            "LoadState=loaded\nActiveState=inactive\nNextElapseUSecRealtime=Sun 2026-08-16 04:30:00 UTC"
                .to_string(),
        ))],
    );

    let status = clock_status_with(root.path(), ClockPlatform::Systemd, &runner)
        .expect("read disabled timer status");

    assert!(!status.enabled);
    assert!(status.loaded);
    assert_eq!(status.running, Some(false));
    assert!(!status.schedulable);
    assert_eq!(status.effective_cadence_seconds, None);
    assert!(status.health_issue.is_none());
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
    let runner = MockRunner::new(vec![
        Ok(true),
        Ok(true),
        Ok(false),
        Ok(true),
        Ok(true),
        Ok(true),
    ]);
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
            "systemctl --user enable orbit-sweep.timer",
            "systemctl --user restart orbit-sweep.timer",
            "systemctl --user daemon-reload",
            "systemctl --user enable orbit-sweep.timer",
            "systemctl --user restart orbit-sweep.timer",
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
