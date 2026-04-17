use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::OrbitError;

pub(crate) fn ensure_dirs(dirs: &[&Path]) -> Result<(), OrbitError> {
    for dir in dirs {
        fs::create_dir_all(dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    }
    Ok(())
}

pub(crate) fn read_child_dirs(dir: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    let mut child_dirs = fs::read_dir(dir)
        .map_err(|e| OrbitError::Io(e.to_string()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    child_dirs.sort();
    Ok(child_dirs)
}

pub(crate) fn list_yaml_files(dir: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut paths = fs::read_dir(dir)
        .map_err(|e| OrbitError::Io(e.to_string()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_yaml(path))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

pub(crate) fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"))
}

pub(crate) fn file_timestamps(path: &Path) -> Result<(DateTime<Utc>, DateTime<Utc>), OrbitError> {
    let metadata = fs::metadata(path).map_err(|e| OrbitError::Io(e.to_string()))?;
    let created_at = metadata.created().ok().map(DateTime::<Utc>::from);
    let updated_at = metadata.modified().ok().map(DateTime::<Utc>::from);
    let now = Utc::now();

    let created_at = created_at.or(updated_at).unwrap_or(now);
    let updated_at = updated_at.unwrap_or(created_at);
    Ok((created_at, updated_at))
}

#[derive(Debug, Clone)]
pub(crate) struct DualLayout {
    pub(crate) primary: PathBuf,
    pub(crate) secondary: PathBuf,
}

impl DualLayout {
    pub(crate) fn ensure(&self) -> Result<(), OrbitError> {
        ensure_dirs(&[self.primary.as_path(), self.secondary.as_path()])
    }

    pub(crate) fn primary_file(&self, stem: &str, ext: &str) -> PathBuf {
        self.primary.join(file_name(stem, ext))
    }

    pub(crate) fn secondary_file(&self, stem: &str, ext: &str) -> PathBuf {
        self.secondary.join(file_name(stem, ext))
    }

    pub(crate) fn locate(&self, stem: &str, ext: &str) -> Option<(PathBuf, bool)> {
        let primary = self.primary_file(stem, ext);
        if primary.exists() {
            return Some((primary, true));
        }

        let secondary = self.secondary_file(stem, ext);
        if secondary.exists() {
            return Some((secondary, false));
        }

        None
    }
}

fn file_name(stem: &str, ext: &str) -> String {
    if ext.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}.{ext}")
    }
}
