use std::path::Path;
use std::process::Command;

use orbit_common::types::OrbitError;
use serde::Serialize;

use crate::commands::active_model;
use crate::vector::{SemanticStats, VectorStore};
use crate::{CompanionPaths, RpcResponse, RpcResult, locate_companion};

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

pub fn run(
    vector_store: &VectorStore,
    task_ids: &[String],
) -> Result<SemanticStatsResult, OrbitError> {
    let rows = vector_store.stats(task_ids)?;
    let companion = companion_status();
    Ok(SemanticStatsResult { rows, companion })
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
