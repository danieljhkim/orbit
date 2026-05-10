use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use orbit_common::types::OrbitError;
use serde::Serialize;

use crate::commands::{DEFAULT_RELEASE_BASE_URL, parse_model};
use crate::{CompanionPaths, RpcResponse, RpcResult, platform_companion_filename};

#[derive(Debug, Clone)]
pub struct SemanticInstallParams {
    pub model: Option<String>,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticInstallResult {
    pub companion_path: String,
    pub companion_changed: bool,
    pub model_id: String,
    pub model_installed: bool,
}

pub fn run(params: SemanticInstallParams) -> Result<SemanticInstallResult, OrbitError> {
    let spec = parse_model(params.model.as_deref())?;
    let paths = CompanionPaths::default_under_home()?;
    fs::create_dir_all(&paths.bin_dir).map_err(|error| OrbitError::Io(error.to_string()))?;
    fs::create_dir_all(&paths.models_dir).map_err(|error| OrbitError::Io(error.to_string()))?;

    let companion_path = paths.companion_path();
    let companion_changed = if params.force || companion_needs_install(&companion_path) {
        install_companion(&companion_path)?;
        true
    } else {
        false
    };

    let model_dir = paths.model_dir(spec.alias);
    let marker_path = model_dir.join("orbit-model.json");
    let model_installed = if marker_path.exists() {
        false
    } else {
        fs::create_dir_all(&model_dir).map_err(|error| OrbitError::Io(error.to_string()))?;
        download_model_with_companion(&companion_path, spec.alias, &model_dir)?;
        true
    };
    fs::write(&paths.active_model_path, spec.alias)
        .map_err(|error| OrbitError::Io(error.to_string()))?;

    Ok(SemanticInstallResult {
        companion_path: companion_path.to_string_lossy().to_string(),
        companion_changed,
        model_id: spec.alias.to_string(),
        model_installed,
    })
}

fn install_companion(destination: &Path) -> Result<(), OrbitError> {
    let temp_path = temporary_companion_path(destination)?;
    if temp_path.exists() {
        fs::remove_file(&temp_path).map_err(|error| OrbitError::Io(error.to_string()))?;
    }

    let install_result = install_companion_to_temp(&temp_path)
        .and_then(|()| replace_companion(&temp_path, destination));
    if install_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    install_result
}

fn install_companion_to_temp(temp_path: &Path) -> Result<(), OrbitError> {
    if let Ok(local_path) = std::env::var("ORBIT_EMBED_COMPANION")
        && Path::new(&local_path).is_file()
    {
        fs::copy(&local_path, temp_path).map_err(|error| OrbitError::Io(error.to_string()))?;
        make_executable(temp_path)?;
        return Ok(());
    }

    let url = std::env::var("ORBIT_EMBED_COMPANION_URL").unwrap_or_else(|_| {
        format!(
            "{DEFAULT_RELEASE_BASE_URL}/{}",
            platform_companion_filename()
        )
    });
    let bytes = reqwest::blocking::get(&url)
        .map_err(|error| OrbitError::Execution(format!("failed to download companion: {error}")))?
        .error_for_status()
        .map_err(|error| OrbitError::Execution(format!("failed to download companion: {error}")))?
        .bytes()
        .map_err(|error| {
            OrbitError::Execution(format!("failed to read companion download: {error}"))
        })?;
    fs::write(temp_path, bytes).map_err(|error| OrbitError::Io(error.to_string()))?;
    make_executable(temp_path)
}

fn companion_needs_install(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    match companion_version(path) {
        Some(version) => version != env!("CARGO_PKG_VERSION"),
        None => true,
    }
}

fn companion_version(path: &Path) -> Option<String> {
    let output = Command::new(path)
        .arg("--version-info")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_version_info(&output.stdout)
}

fn parse_version_info(stdout: &[u8]) -> Option<String> {
    let output = std::str::from_utf8(stdout).ok()?;
    output.lines().find_map(|line| {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        match serde_json::from_str::<RpcResponse>(line).ok()? {
            RpcResponse::Result {
                result:
                    RpcResult::Info {
                        version: Some(version),
                        ..
                    },
                ..
            } => Some(version),
            _ => None,
        }
    })
}

fn temporary_companion_path(destination: &Path) -> Result<std::path::PathBuf, OrbitError> {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "companion destination has no file name: {}",
                destination.display()
            ))
        })?;
    Ok(destination.with_file_name(format!(".{file_name}.tmp-{}", std::process::id())))
}

#[cfg(unix)]
fn replace_companion(temp_path: &Path, destination: &Path) -> Result<(), OrbitError> {
    fs::rename(temp_path, destination).map_err(|error| OrbitError::Io(error.to_string()))
}

#[cfg(not(unix))]
fn replace_companion(temp_path: &Path, destination: &Path) -> Result<(), OrbitError> {
    if destination.exists() {
        fs::remove_file(destination).map_err(|error| OrbitError::Io(error.to_string()))?;
    }
    fs::rename(temp_path, destination).map_err(|error| OrbitError::Io(error.to_string()))
}

fn download_model_with_companion(
    companion_path: &Path,
    model: &str,
    model_dir: &Path,
) -> Result<(), OrbitError> {
    let status = Command::new(companion_path)
        .arg("--model")
        .arg(model)
        .arg("--model-path")
        .arg(model_dir)
        .arg("--download-model")
        .status()
        .map_err(|error| {
            OrbitError::Execution(format!(
                "failed to run embedding companion for model download: {error}"
            ))
        })?;
    if !status.success() {
        return Err(OrbitError::Execution(format!(
            "embedding companion failed to download model `{model}`"
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), OrbitError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|error| OrbitError::Io(error.to_string()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).map_err(|error| OrbitError::Io(error.to_string()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), OrbitError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
            set_env("ORBIT_EMBED_COMPANION", &source_path.to_string_lossy());
            remove_env("ORBIT_EMBED_COMPANION_URL");

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
                "ORBIT_EMBED_COMPANION",
                "ORBIT_EMBED_COMPANION_URL",
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
}
