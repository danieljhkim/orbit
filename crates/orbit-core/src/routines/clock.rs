//! OS clock integration [ORB-10021] / ADR-0204: the OS owns the wake-up,
//! Orbit owns everything else. `orbit routine init --install-clock` renders
//! the platform unit from the templates in `assets/clock/` and installs it
//! as a per-user unit (launchd agent on macOS, systemd user timer on Linux).
//! There is no resident Orbit daemon.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use orbit_common::types::OrbitError;

const LAUNCHD_PLIST_TEMPLATE: &str = include_str!("../../assets/clock/com.orbit.sweep.plist");
const SYSTEMD_SERVICE_TEMPLATE: &str = include_str!("../../assets/clock/orbit-sweep.service");
const SYSTEMD_TIMER_TEMPLATE: &str = include_str!("../../assets/clock/orbit-sweep.timer");

/// launchd agent label (macOS).
pub const LAUNCHD_LABEL: &str = "com.orbit.sweep";
/// systemd unit base name (Linux).
pub const SYSTEMD_UNIT: &str = "orbit-sweep";

/// What an installation attempt did: files written, plus either a successful
/// activation or the commands the user must run themselves.
#[derive(Debug)]
pub struct ClockInstallReport {
    /// Unit files written.
    pub files_written: Vec<PathBuf>,
    /// True when the unit was activated (loaded/enabled) successfully.
    pub activated: bool,
    /// Follow-up commands to run manually when activation failed or was
    /// unavailable (e.g. no user systemd session).
    pub manual_steps: Vec<String>,
}

/// Render and install the platform clock unit for the current user, then try
/// to activate it. File writes are mandatory (errors propagate); activation
/// is best-effort with explicit manual steps on failure, because unit
/// managers behave differently across sessions (SSH, headless, containers).
pub fn install_clock(global_root: &Path) -> Result<ClockInstallReport, OrbitError> {
    let orbit_bin = std::env::current_exe()
        .map_err(|error| OrbitError::Io(format!("resolve current orbit executable: {error}")))?;
    let orbit_bin = orbit_bin.to_string_lossy().to_string();

    if cfg!(target_os = "macos") {
        install_launchd(global_root, &orbit_bin)
    } else {
        install_systemd(&orbit_bin)
    }
}

fn install_launchd(global_root: &Path, orbit_bin: &str) -> Result<ClockInstallReport, OrbitError> {
    let home = home_dir()?;
    let log_path = global_root.join("logs").join("sweep.log");
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|error| OrbitError::Io(error.to_string()))?;
    }
    let plist = LAUNCHD_PLIST_TEMPLATE
        .replace("{{ORBIT_BIN}}", orbit_bin)
        .replace("{{LOG_PATH}}", &log_path.to_string_lossy());

    let agents_dir = home.join("Library/LaunchAgents");
    fs::create_dir_all(&agents_dir).map_err(|error| OrbitError::Io(error.to_string()))?;
    let plist_path = agents_dir.join(format!("{LAUNCHD_LABEL}.plist"));
    fs::write(&plist_path, plist).map_err(|error| {
        OrbitError::Io(format!(
            "failed to write '{}': {error}",
            plist_path.display()
        ))
    })?;

    // `launchctl load` is deprecated but still the most portable activation;
    // a stale agent is unloaded first so re-installs pick up the new binary
    // path.
    let _ = Command::new("launchctl")
        .args(["unload", &plist_path.to_string_lossy()])
        .output();
    let activated = Command::new("launchctl")
        .args(["load", &plist_path.to_string_lossy()])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);

    let manual_steps = if activated {
        Vec::new()
    } else {
        vec![format!("launchctl load {}", plist_path.display())]
    };
    Ok(ClockInstallReport {
        files_written: vec![plist_path],
        activated,
        manual_steps,
    })
}

fn install_systemd(orbit_bin: &str) -> Result<ClockInstallReport, OrbitError> {
    let home = home_dir()?;
    let unit_dir = home.join(".config/systemd/user");
    fs::create_dir_all(&unit_dir).map_err(|error| OrbitError::Io(error.to_string()))?;

    let service_path = unit_dir.join(format!("{SYSTEMD_UNIT}.service"));
    let timer_path = unit_dir.join(format!("{SYSTEMD_UNIT}.timer"));
    fs::write(
        &service_path,
        SYSTEMD_SERVICE_TEMPLATE.replace("{{ORBIT_BIN}}", orbit_bin),
    )
    .map_err(|error| {
        OrbitError::Io(format!(
            "failed to write '{}': {error}",
            service_path.display()
        ))
    })?;
    fs::write(&timer_path, SYSTEMD_TIMER_TEMPLATE).map_err(|error| {
        OrbitError::Io(format!(
            "failed to write '{}': {error}",
            timer_path.display()
        ))
    })?;

    let reloaded = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    let activated = reloaded
        && Command::new("systemctl")
            .args([
                "--user",
                "enable",
                "--now",
                &format!("{SYSTEMD_UNIT}.timer"),
            ])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);

    let manual_steps = if activated {
        Vec::new()
    } else {
        vec![
            "systemctl --user daemon-reload".to_string(),
            format!("systemctl --user enable --now {SYSTEMD_UNIT}.timer"),
        ]
    };
    Ok(ClockInstallReport {
        files_written: vec![service_path, timer_path],
        activated,
        manual_steps,
    })
}

fn home_dir() -> Result<PathBuf, OrbitError> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| OrbitError::InvalidInput("HOME is not set".to_string()))
}
