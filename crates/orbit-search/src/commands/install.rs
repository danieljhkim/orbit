use std::fs;
use std::io::Write;
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
    emit_stale_companion_hint(&paths);

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
    if let Ok(local_path) = std::env::var("ORBIT_SEARCH_COMPANION")
        && Path::new(&local_path).is_file()
    {
        fs::copy(&local_path, temp_path).map_err(|error| OrbitError::Io(error.to_string()))?;
        make_executable(temp_path)?;
        return Ok(());
    }

    let url = std::env::var("ORBIT_SEARCH_COMPANION_URL").unwrap_or_else(|_| {
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

fn emit_stale_companion_hint(paths: &CompanionPaths) {
    let stale_path = paths.bin_dir.join(legacy_platform_companion_filename());
    if stale_path.exists() {
        let _ = writeln!(
            std::io::stderr().lock(),
            "stale companion detected at {}; remove it or run `orbit semantic install --force`",
            stale_path.display()
        );
    }
}

fn legacy_platform_companion_filename() -> String {
    let base = concat!("orbit-", "embed", "-companion");
    if cfg!(windows) {
        format!("{base}-{}.exe", crate::platform_id())
    } else {
        format!("{base}-{}", crate::platform_id())
    }
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
                "failed to run search companion for model download: {error}"
            ))
        })?;
    if !status.success() {
        return Err(OrbitError::Execution(format!(
            "search companion failed to download model `{model}`"
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
