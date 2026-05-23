use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use orbit_common::types::OrbitError;

use super::constants::{LEARNING_DOC_FILE_EXT, SUPERSEDED_DIR_NAME};
use super::layout::{learning_doc_path, validate_learning_id};

const MIGRATION_COMMAND: &str = "orbit learning migrate-layout";
const WORKSPACE_LOCK_RELATIVE_PATH: &str = "state/workspace.lock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningLayoutMigrationReport {
    pub already_migrated: bool,
    pub moved_active: usize,
    pub moved_superseded: usize,
    pub removed_superseded_dir: bool,
}

impl LearningLayoutMigrationReport {
    pub fn moved_total(&self) -> usize {
        self.moved_active + self.moved_superseded
    }
}

pub(crate) fn reject_legacy_flat_layout(root: &Path) -> Result<(), OrbitError> {
    if has_legacy_flat_layout(root)? {
        return Err(OrbitError::Migration(format!(
            "workspace uses the legacy flat learning layout; run `{MIGRATION_COMMAND}`"
        )));
    }
    Ok(())
}

pub fn migrate_learning_layout(
    root: &Path,
    workspace_orbit_dir: &Path,
) -> Result<LearningLayoutMigrationReport, OrbitError> {
    let active = legacy_flat_learning_paths(root)?;
    let superseded_dir = root.join(SUPERSEDED_DIR_NAME);
    let superseded = legacy_superseded_learning_paths(&superseded_dir)?;

    if active.is_empty() && superseded.is_empty() && !superseded_dir.exists() {
        return Ok(LearningLayoutMigrationReport {
            already_migrated: true,
            moved_active: 0,
            moved_superseded: 0,
            removed_superseded_dir: false,
        });
    }

    let _workspace_lock = acquire_workspace_migration_lock(workspace_orbit_dir)?;

    let mut moved_active = 0;
    for path in active {
        let id = learning_id_from_flat_path(&path)?;
        move_flat_learning_file(&path, root, &id)?;
        moved_active += 1;
    }

    let mut moved_superseded = 0;
    for path in superseded {
        let id = learning_id_from_flat_path(&path)?;
        move_flat_learning_file(&path, root, &id)?;
        moved_superseded += 1;
    }

    let removed_superseded_dir = remove_superseded_dir_if_present(&superseded_dir)?;

    Ok(LearningLayoutMigrationReport {
        already_migrated: false,
        moved_active,
        moved_superseded,
        removed_superseded_dir,
    })
}

fn has_legacy_flat_layout(root: &Path) -> Result<bool, OrbitError> {
    Ok(!legacy_flat_learning_paths(root)?.is_empty()
        || !legacy_superseded_learning_paths(&root.join(SUPERSEDED_DIR_NAME))?.is_empty())
}

fn legacy_flat_learning_paths(root: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(root).map_err(|error| OrbitError::Io(error.to_string()))? {
        let entry = entry.map_err(|error| OrbitError::Io(error.to_string()))?;
        if !entry
            .file_type()
            .map_err(|error| OrbitError::Io(error.to_string()))?
            .is_file()
        {
            continue;
        }
        let path = entry.path();
        if let Some(id) = path.file_stem().and_then(|value| value.to_str())
            && validate_learning_id(id).is_ok()
            && path.extension().and_then(|value| value.to_str()) == Some(LEARNING_DOC_FILE_EXT)
        {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn legacy_superseded_learning_paths(superseded_dir: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    let mut out = Vec::new();
    if !superseded_dir.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(superseded_dir).map_err(|error| OrbitError::Io(error.to_string()))? {
        let entry = entry.map_err(|error| OrbitError::Io(error.to_string()))?;
        if !entry
            .file_type()
            .map_err(|error| OrbitError::Io(error.to_string()))?
            .is_file()
        {
            continue;
        }
        let path = entry.path();
        if let Some(id) = path.file_stem().and_then(|value| value.to_str())
            && validate_learning_id(id).is_ok()
            && path.extension().and_then(|value| value.to_str()) == Some(LEARNING_DOC_FILE_EXT)
        {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn learning_id_from_flat_path(path: &Path) -> Result<String, OrbitError> {
    let id = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| OrbitError::Store(format!("invalid learning path '{}'", path.display())))?
        .to_string();
    validate_learning_id(&id)?;
    Ok(id)
}

fn move_flat_learning_file(source: &Path, root: &Path, id: &str) -> Result<(), OrbitError> {
    move_flat_learning_file_to_target(source, &learning_doc_path(root, id))
}

fn move_flat_learning_file_to_target(source: &Path, target: &Path) -> Result<(), OrbitError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| OrbitError::Io(error.to_string()))?;
    }
    if target.exists() {
        let source_bytes = fs::read(source).map_err(|error| OrbitError::Io(error.to_string()))?;
        let target_bytes = fs::read(target).map_err(|error| OrbitError::Io(error.to_string()))?;
        if source_bytes == target_bytes {
            fs::remove_file(source).map_err(|error| OrbitError::Io(error.to_string()))?;
            return Ok(());
        }
        return Err(OrbitError::Migration(format!(
            "cannot migrate '{}': destination '{}' already exists with different content",
            source.display(),
            target.display()
        )));
    }
    fs::rename(source, target).map_err(|error| OrbitError::Io(error.to_string()))
}

fn remove_superseded_dir_if_present(superseded_dir: &Path) -> Result<bool, OrbitError> {
    match fs::remove_dir(superseded_dir) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(OrbitError::Migration(format!(
            "failed to remove legacy superseded directory '{}': {error}",
            superseded_dir.display()
        ))),
    }
}

fn acquire_workspace_migration_lock(workspace_orbit_dir: &Path) -> Result<File, OrbitError> {
    let lock_path = workspace_orbit_dir.join(WORKSPACE_LOCK_RELATIVE_PATH);
    let parent = lock_path.parent().ok_or_else(|| {
        OrbitError::WorkspaceError(format!(
            "cannot determine workspace lock parent for '{}'",
            lock_path.display()
        ))
    })?;
    fs::create_dir_all(parent).map_err(|error| OrbitError::Io(error.to_string()))?;

    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    if let Err(error) = file.try_lock_exclusive() {
        let owner = read_lock_owner(&mut file).unwrap_or_else(|| "unknown process".to_string());
        return Err(OrbitError::WorkspaceError(format!(
            "cannot run `{MIGRATION_COMMAND}` while the workspace lock is held by {owner} at '{}': {error}",
            lock_path.display()
        )));
    }

    file.set_len(0)
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    writeln!(file, "pid={}", std::process::id())
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    file.sync_all()
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    Ok(file)
}

fn read_lock_owner(file: &mut File) -> Option<String> {
    let mut raw = String::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_to_string(&mut raw).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(format!("process {trimmed}"))
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests;
