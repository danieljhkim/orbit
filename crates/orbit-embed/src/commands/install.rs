use std::fs;
use std::path::Path;
use std::process::Command;

use orbit_common::types::OrbitError;
use serde::Serialize;

use crate::commands::{DEFAULT_RELEASE_BASE_URL, parse_model};
use crate::{CompanionPaths, platform_companion_filename};

#[derive(Debug, Clone)]
pub struct SemanticInstallParams {
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticInstallResult {
    pub companion_path: String,
    pub companion_installed: bool,
    pub model_id: String,
    pub model_installed: bool,
}

pub fn run(params: SemanticInstallParams) -> Result<SemanticInstallResult, OrbitError> {
    let spec = parse_model(params.model.as_deref())?;
    let paths = CompanionPaths::default_under_home()?;
    fs::create_dir_all(&paths.bin_dir).map_err(|error| OrbitError::Io(error.to_string()))?;
    fs::create_dir_all(&paths.models_dir).map_err(|error| OrbitError::Io(error.to_string()))?;

    let companion_path = paths.companion_path();
    let companion_installed = if companion_path.exists() {
        false
    } else {
        install_companion(&companion_path)?;
        true
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
        companion_installed,
        model_id: spec.alias.to_string(),
        model_installed,
    })
}

fn install_companion(destination: &Path) -> Result<(), OrbitError> {
    if let Ok(local_path) = std::env::var("ORBIT_EMBED_COMPANION")
        && Path::new(&local_path).is_file()
    {
        fs::copy(&local_path, destination).map_err(|error| OrbitError::Io(error.to_string()))?;
        make_executable(destination)?;
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
    fs::write(destination, bytes).map_err(|error| OrbitError::Io(error.to_string()))?;
    make_executable(destination)
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
