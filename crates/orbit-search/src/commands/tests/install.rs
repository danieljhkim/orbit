//! Unit tests for `install` — sibling layout under commands/tests/.

use super::super::install::{SemanticInstallParams, run};

use crate::CompanionPaths;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use tempfile::{TempDir, tempdir};

#[test]
#[cfg(unix)]
fn stale_installed_companion_is_replaced_and_reported_as_changed() {
    let _guard = EnvGuard::new();
    let fixture = InstallFixture::new();
    fixture.write_installed_companion("0.3.1", "old");
    fixture.write_source_companion(env!("CARGO_PKG_VERSION"), "fresh");

    let result = run(SemanticInstallParams {
        model: None,
        force: false,
    })
    .expect("install should replace stale companion");

    assert!(result.companion_changed);
    assert!(!result.model_installed);
    assert!(
        std::fs::read_to_string(fixture.paths.companion_path())
            .expect("read replaced companion")
            .contains("replacement-marker: fresh")
    );
    let json = serde_json::to_value(&result).expect("serialize result");
    assert_eq!(json["companion_changed"], true);
    assert_eq!(json.get("companion_installed"), None);
}

#[test]
#[cfg(unix)]
fn force_replaces_current_companion() {
    let _guard = EnvGuard::new();
    let fixture = InstallFixture::new();
    fixture.write_installed_companion(env!("CARGO_PKG_VERSION"), "old-current");
    fixture.write_source_companion(env!("CARGO_PKG_VERSION"), "forced-fresh");

    let result = run(SemanticInstallParams {
        model: None,
        force: true,
    })
    .expect("forced install should replace current companion");

    assert!(result.companion_changed);
    assert!(
        std::fs::read_to_string(fixture.paths.companion_path())
            .expect("read replaced companion")
            .contains("replacement-marker: forced-fresh")
    );
}

#[test]
#[cfg(unix)]
fn current_companion_is_left_in_place_without_force() {
    let _guard = EnvGuard::new();
    let fixture = InstallFixture::new();
    fixture.write_installed_companion(env!("CARGO_PKG_VERSION"), "kept-current");
    fixture.write_source_companion(env!("CARGO_PKG_VERSION"), "unused-source");

    let result = run(SemanticInstallParams {
        model: None,
        force: false,
    })
    .expect("current install should be accepted");

    assert!(!result.companion_changed);
    assert!(
        std::fs::read_to_string(fixture.paths.companion_path())
            .expect("read kept companion")
            .contains("replacement-marker: kept-current")
    );
}

struct InstallFixture {
    _temp: TempDir,
    paths: CompanionPaths,
    source_path: PathBuf,
}

impl InstallFixture {
    fn new() -> Self {
        let temp = tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let source_path = temp.path().join("source-companion");
        std::fs::create_dir_all(&home).expect("create home");
        set_env("HOME", &home.to_string_lossy());
        set_env("USERPROFILE", &home.to_string_lossy());
        set_env("ORBIT_SEARCH_COMPANION", &source_path.to_string_lossy());
        remove_env("ORBIT_SEARCH_COMPANION_URL");

        let paths = CompanionPaths::default_under_home().expect("paths");
        std::fs::create_dir_all(&paths.bin_dir).expect("create bin");
        let model_dir = paths.model_dir(crate::default_model().alias);
        std::fs::create_dir_all(&model_dir).expect("create model dir");
        std::fs::write(model_dir.join("orbit-model.json"), "{}").expect("write marker");

        Self {
            _temp: temp,
            paths,
            source_path,
        }
    }

    #[cfg(unix)]
    fn write_installed_companion(&self, version: &str, marker: &str) {
        write_mock_companion(&self.paths.companion_path(), version, marker);
    }

    #[cfg(unix)]
    fn write_source_companion(&self, version: &str, marker: &str) {
        write_mock_companion(&self.source_path, version, marker);
    }
}

struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    vars: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn new() -> Self {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let lock = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let names = [
            "HOME",
            "USERPROFILE",
            "ORBIT_SEARCH_COMPANION",
            "ORBIT_SEARCH_COMPANION_URL",
        ];
        let vars = names
            .into_iter()
            .map(|name| (name, std::env::var(name).ok()))
            .collect::<Vec<_>>();
        Self { _lock: lock, vars }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in &self.vars {
            match value {
                Some(value) => set_env(name, value),
                None => remove_env(name),
            }
        }
    }
}

#[cfg(unix)]
fn write_mock_companion(path: &std::path::Path, version: &str, marker: &str) {
    use std::os::unix::fs::PermissionsExt;

    let script = format!(
        r#"#!/bin/sh
# replacement-marker: {marker}
if [ "$1" = "--version-info" ]; then
  printf '%s\n' '{{"id":0,"result":{{"model_id":"bge-small-en-v1.5","dim":0,"max_input_tokens":0,"version":"{version}"}}}}'
  exit 0
fi
exit 0
"#
    );
    std::fs::write(path, script).expect("write companion");
    let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("chmod companion");
}

fn set_env(name: &str, value: &str) {
    // SAFETY: tests that mutate process environment hold EnvGuard's global lock.
    unsafe { std::env::set_var(name, value) }
}

fn remove_env(name: &str) {
    // SAFETY: tests that mutate process environment hold EnvGuard's global lock.
    unsafe { std::env::remove_var(name) }
}
