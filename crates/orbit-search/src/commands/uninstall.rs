use std::fs;

use orbit_common::types::OrbitError;
use serde::Serialize;

use crate::CompanionPaths;
use crate::commands::{active_model, remove_file_if_exists};
use crate::{ModelSpec, default_model};

#[derive(Debug, Clone)]
pub struct SemanticUninstallParams {
    pub model: Option<String>,
    pub all: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticUninstallResult {
    pub removed_companion: bool,
    pub removed_models: Vec<String>,
}

pub fn run(params: SemanticUninstallParams) -> Result<SemanticUninstallResult, OrbitError> {
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
