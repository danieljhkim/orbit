use std::fs;
use std::path::{Path, PathBuf};

use orbit_types::OrbitError;
use serde::{Serialize, de::DeserializeOwned};

use super::fs_utils::write_atomic;
use super::layout::list_yaml_files;

pub(crate) fn read_yaml<T: DeserializeOwned>(path: &Path, label: &str) -> Result<T, OrbitError> {
    read_yaml_with(path, |path, err| {
        OrbitError::Store(format!("invalid {label} '{}': {err}", path.display()))
    })
}

pub(crate) fn read_yaml_with<T: DeserializeOwned, F>(
    path: &Path,
    invalid: F,
) -> Result<T, OrbitError>
where
    F: FnOnce(&Path, serde_yaml::Error) -> OrbitError,
{
    let raw = fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
    serde_yaml::from_str(&raw).map_err(|err| invalid(path, err))
}

pub(crate) fn write_yaml_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), OrbitError> {
    write_yaml_atomic_with(path, value, |value| {
        serde_yaml::to_string(value).map_err(|e| OrbitError::Store(e.to_string()))
    })
}

pub(crate) fn write_yaml_atomic_with<T, F>(
    path: &Path,
    value: &T,
    serialize: F,
) -> Result<(), OrbitError>
where
    F: FnOnce(&T) -> Result<String, OrbitError>,
{
    let yaml = serialize(value)?;
    write_atomic(path, &yaml)
}

pub(crate) fn enumerate_yaml<T, F>(
    dir: &Path,
    _label: &str,
    mut transform: F,
) -> Result<Vec<T>, OrbitError>
where
    F: FnMut(PathBuf) -> Result<T, OrbitError>,
{
    let mut items = Vec::new();
    for path in list_yaml_files(dir)? {
        items.push(transform(path)?);
    }
    Ok(items)
}
