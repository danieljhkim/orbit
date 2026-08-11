use std::fs;
use std::path::Path;

use tempfile::tempdir;

use super::super::clock::{ClockSettings, render_systemd_service, render_systemd_timer};

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
