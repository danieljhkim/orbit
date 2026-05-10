use std::fs;
use std::path::Path;
use std::process::Command;

use orbit_common::types::OrbitError;
use orbit_embed::{
    CompanionPaths, Embedder, ModelSpec, RpcResponse, RpcResult, SubprocessEmbedder, default_model,
    locate_companion, platform_companion_filename,
};
use orbit_store::{SemanticStats, UpsertReport};
use serde::Serialize;

use crate::OrbitRuntime;

const DEFAULT_RELEASE_BASE_URL: &str =
    "https://github.com/danieljhkim/orbit/releases/latest/download";

#[derive(Debug, Clone)]
pub struct SemanticInstallParams {
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SemanticUninstallParams {
    pub model: Option<String>,
    pub all: bool,
}

#[derive(Debug, Clone)]
pub struct SemanticReindexParams {
    pub model: Option<String>,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticInstallResult {
    pub companion_path: String,
    pub companion_installed: bool,
    pub model_id: String,
    pub model_installed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticUninstallResult {
    pub removed_companion: bool,
    pub removed_models: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticReindexResult {
    pub model_id: String,
    pub report: UpsertReport,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticStatsResult {
    pub rows: SemanticStats,
    pub companion: CompanionStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompanionStatus {
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub model: Option<String>,
}

impl OrbitRuntime {
    pub fn semantic_install(
        &self,
        params: SemanticInstallParams,
    ) -> Result<SemanticInstallResult, OrbitError> {
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

    pub fn semantic_uninstall(
        &self,
        params: SemanticUninstallParams,
    ) -> Result<SemanticUninstallResult, OrbitError> {
        let paths = CompanionPaths::default_under_home()?;
        if params.all {
            let removed_companion = remove_file_if_exists(&paths.companion_path())?;
            let mut removed_models = Vec::new();
            if paths.models_dir.exists() {
                for entry in fs::read_dir(&paths.models_dir)
                    .map_err(|error| OrbitError::Io(error.to_string()))?
                {
                    let entry = entry.map_err(|error| OrbitError::Io(error.to_string()))?;
                    if entry.path().is_dir() {
                        removed_models.push(entry.file_name().to_string_lossy().to_string());
                    }
                }
                fs::remove_dir_all(&paths.models_dir)
                    .map_err(|error| OrbitError::Io(error.to_string()))?;
            }
            let _ = remove_file_if_exists(&paths.active_model_path)?;
            return Ok(SemanticUninstallResult {
                removed_companion,
                removed_models,
            });
        }

        let model = match params.model {
            Some(model) => ModelSpec::parse(&model)?.alias.to_string(),
            None => active_model(&paths).unwrap_or_else(|| default_model().alias.to_string()),
        };
        let model_dir = paths.model_dir(&model);
        let removed = if model_dir.exists() {
            fs::remove_dir_all(&model_dir).map_err(|error| OrbitError::Io(error.to_string()))?;
            true
        } else {
            false
        };
        if active_model(&paths).as_deref() == Some(model.as_str()) {
            let _ = remove_file_if_exists(&paths.active_model_path)?;
        }

        Ok(SemanticUninstallResult {
            removed_companion: false,
            removed_models: if removed { vec![model] } else { Vec::new() },
        })
    }

    pub fn semantic_reindex(
        &self,
        params: SemanticReindexParams,
    ) -> Result<SemanticReindexResult, OrbitError> {
        let model = parse_model(params.model.as_deref())?;
        let embedder = SubprocessEmbedder::with_model(model.alias)?;
        let tasks = self.stores().tasks().list()?;
        let report =
            self.stores()
                .semantic_vector
                .reindex_tasks(&tasks, &embedder, params.force)?;
        Ok(SemanticReindexResult {
            model_id: embedder.model_id().to_string(),
            report,
        })
    }

    pub fn semantic_stats(&self) -> Result<SemanticStatsResult, OrbitError> {
        let task_ids = self
            .stores()
            .tasks()
            .list()?
            .into_iter()
            .map(|task| task.id)
            .collect::<Vec<_>>();
        let rows = self.stores().semantic_vector.stats(&task_ids)?;
        let companion = companion_status();
        Ok(SemanticStatsResult { rows, companion })
    }
}

fn parse_model(model: Option<&str>) -> Result<ModelSpec, OrbitError> {
    match model {
        Some(value) => ModelSpec::parse(value),
        None => Ok(default_model()),
    }
}

fn active_model(paths: &CompanionPaths) -> Option<String> {
    fs::read_to_string(&paths.active_model_path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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

fn remove_file_if_exists(path: &Path) -> Result<bool, OrbitError> {
    if path.exists() {
        fs::remove_file(path).map_err(|error| OrbitError::Io(error.to_string()))?;
        Ok(true)
    } else {
        Ok(false)
    }
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

fn companion_status() -> CompanionStatus {
    let path = locate_companion().ok();
    let Some(path) = path else {
        return CompanionStatus {
            installed: false,
            path: None,
            version: None,
            model: CompanionPaths::default_under_home()
                .ok()
                .and_then(|paths| active_model(&paths)),
        };
    };
    let version = companion_version(&path).ok();
    let model = CompanionPaths::default_under_home()
        .ok()
        .and_then(|paths| active_model(&paths));
    CompanionStatus {
        installed: true,
        path: Some(path.to_string_lossy().to_string()),
        version,
        model,
    }
}

fn companion_version(path: &Path) -> Result<String, OrbitError> {
    let output = Command::new(path)
        .arg("--version-info")
        .output()
        .map_err(|error| OrbitError::Execution(error.to_string()))?;
    if !output.status.success() {
        return Err(OrbitError::Execution(
            "companion version check failed".to_string(),
        ));
    }
    let line = String::from_utf8(output.stdout)
        .map_err(|error| OrbitError::Execution(error.to_string()))?;
    let response: RpcResponse =
        serde_json::from_str(&line).map_err(|error| OrbitError::Execution(error.to_string()))?;
    match response {
        RpcResponse::Result {
            result:
                RpcResult::Info {
                    version: Some(version),
                    ..
                },
            ..
        } => Ok(version),
        _ => Err(OrbitError::Execution(
            "companion version response was malformed".to_string(),
        )),
    }
}
