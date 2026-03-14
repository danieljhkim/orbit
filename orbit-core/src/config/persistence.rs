use std::path::{Path, PathBuf};

use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::paths;

use super::raw::{RawPersistenceConfig, RawRuntimeConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistenceType {
    File,
    Sqlite,
}

#[derive(Debug, Clone)]
pub(crate) struct EntityPersistenceConfig {
    pub(crate) persistence_type: PersistenceType,
    pub(crate) path: PathBuf,
    pub(crate) format: Option<String>,
}

impl EntityPersistenceConfig {
    fn to_json_value(&self) -> Value {
        json!({
            "type": match self.persistence_type {
                PersistenceType::File => "file",
                PersistenceType::Sqlite => "sqlite",
            },
            "path": self.path.to_string_lossy(),
            "format": self.format,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PersistenceConfig {
    pub(crate) job: EntityPersistenceConfig,
    pub(crate) activity: EntityPersistenceConfig,
    pub(crate) skill: PathBuf,
    pub(crate) task: PathBuf,
    pub(crate) audit: EntityPersistenceConfig,
}

impl PersistenceConfig {
    pub(crate) fn default_for_data_root(data_root: &Path) -> Self {
        let sqlite_default = data_root.join("orbit.db");
        Self {
            job: EntityPersistenceConfig {
                persistence_type: PersistenceType::File,
                path: data_root.join("jobs"),
                format: Some("yaml".to_string()),
            },
            activity: EntityPersistenceConfig {
                persistence_type: PersistenceType::File,
                path: data_root.join("activities"),
                format: Some("yaml".to_string()),
            },
            skill: data_root.join("skills"),
            task: data_root.join("tasks"),
            audit: EntityPersistenceConfig {
                persistence_type: PersistenceType::Sqlite,
                path: sqlite_default,
                format: None,
            },
        }
    }

    pub(super) fn from_raw(data_root: &Path, raw: &RawRuntimeConfig) -> Result<Self, OrbitError> {
        let defaults = Self::default_for_data_root(data_root);
        if raw.watch.is_some() {
            return Err(OrbitError::InvalidInput(
                "watch config is no longer supported; remove the [watch] section from config.toml"
                    .to_string(),
            ));
        }

        let skill = resolve_path_only_entity(
            raw.skill.as_ref().and_then(|v| v.persistence.as_ref()),
            &defaults.skill,
            data_root,
        )?;

        let task = resolve_path_only_entity(
            raw.task.as_ref().and_then(|v| v.persistence.as_ref()),
            &defaults.task,
            data_root,
        )?;

        Ok(Self {
            job: parse_configurable_entity(
                "job",
                raw.job.as_ref().and_then(|v| v.persistence.as_ref()),
                &defaults.job,
                false,
                "yaml",
                data_root,
            )?,
            activity: parse_configurable_entity(
                "activity",
                raw.activity.as_ref().and_then(|v| v.persistence.as_ref()),
                &defaults.activity,
                false,
                "yaml",
                data_root,
            )?,
            skill,
            task,
            audit: parse_sqlite_only_entity(
                "audit",
                raw.audit.as_ref().and_then(|v| v.persistence.as_ref()),
                &defaults.audit,
                data_root,
            )?,
        })
    }

    pub(crate) fn as_json_value(&self) -> Value {
        json!({
            "job": { "persistence": self.job.to_json_value() },
            "activity": { "persistence": self.activity.to_json_value() },
            "skill": { "path": self.skill.to_string_lossy() },
            "task": { "path": self.task.to_string_lossy() },
            "audit": { "persistence": self.audit.to_json_value() },
        })
    }
}

fn resolve_path_only_entity(
    raw: Option<&RawPersistenceConfig>,
    default_path: &Path,
    base_dir: &Path,
) -> Result<PathBuf, OrbitError> {
    let Some(raw) = raw else {
        return Ok(default_path.to_path_buf());
    };
    paths::resolve_config_path(
        raw.path.as_deref(),
        default_path,
        base_dir,
        "persistence.path",
    )
}

fn parse_configurable_entity(
    entity: &str,
    raw: Option<&RawPersistenceConfig>,
    defaults: &EntityPersistenceConfig,
    allow_sqlite: bool,
    required_file_format: &str,
    base_dir: &Path,
) -> Result<EntityPersistenceConfig, OrbitError> {
    let Some(raw) = raw else {
        return Ok(defaults.clone());
    };
    let persistence_type = parse_persistence_type(raw.persistence_type.as_deref(), entity)?;
    if !allow_sqlite && persistence_type == PersistenceType::Sqlite {
        return Err(OrbitError::InvalidInput(format!(
            "{entity}.persistence.type only supports 'file'"
        )));
    }

    match persistence_type {
        PersistenceType::File => {
            let format = raw
                .format
                .as_deref()
                .unwrap_or(required_file_format)
                .to_ascii_lowercase();
            if format != required_file_format {
                return Err(OrbitError::InvalidInput(format!(
                    "{entity}.persistence.format must be '{required_file_format}' for file persistence"
                )));
            }
            Ok(EntityPersistenceConfig {
                persistence_type,
                path: paths::resolve_config_path(
                    raw.path.as_deref(),
                    &defaults.path,
                    base_dir,
                    "persistence.path",
                )?,
                format: Some(format),
            })
        }
        PersistenceType::Sqlite => {
            if raw.format.is_some() {
                return Err(OrbitError::InvalidInput(format!(
                    "{entity}.persistence.format is not supported for sqlite persistence"
                )));
            }
            Ok(EntityPersistenceConfig {
                persistence_type,
                path: paths::resolve_config_path(
                    raw.path.as_deref(),
                    &defaults.path,
                    base_dir,
                    "persistence.path",
                )?,
                format: None,
            })
        }
    }
}

fn parse_sqlite_only_entity(
    entity: &str,
    raw: Option<&RawPersistenceConfig>,
    defaults: &EntityPersistenceConfig,
    base_dir: &Path,
) -> Result<EntityPersistenceConfig, OrbitError> {
    let Some(raw) = raw else {
        return Ok(defaults.clone());
    };
    let persistence_type = match raw.persistence_type.as_deref() {
        None => PersistenceType::Sqlite,
        Some(value) => parse_persistence_type(Some(value), entity)?,
    };
    if persistence_type != PersistenceType::Sqlite {
        return Err(OrbitError::InvalidInput(format!(
            "{entity}.persistence.type only supports 'sqlite'"
        )));
    }
    if raw.format.is_some() {
        return Err(OrbitError::InvalidInput(format!(
            "{entity}.persistence.format is not supported for sqlite persistence"
        )));
    }

    Ok(EntityPersistenceConfig {
        persistence_type,
        path: paths::resolve_config_path(
            raw.path.as_deref(),
            &defaults.path,
            base_dir,
            "persistence.path",
        )?,
        format: None,
    })
}

fn parse_persistence_type(raw: Option<&str>, entity: &str) -> Result<PersistenceType, OrbitError> {
    let value = raw.unwrap_or("file").trim().to_ascii_lowercase();
    match value.as_str() {
        "file" => Ok(PersistenceType::File),
        "sqlite" => Ok(PersistenceType::Sqlite),
        other => Err(OrbitError::InvalidInput(format!(
            "{entity}.persistence.type must be 'file' or 'sqlite' (got '{other}')"
        ))),
    }
}
