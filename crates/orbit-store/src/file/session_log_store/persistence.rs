//! Locked JSONL persistence for [`super::SessionLogStore`].

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::OrbitError;
use orbit_common::utility::fs::{atomic_write_bytes, sync_parent_dir};

use super::{SessionLogAppendParams, SessionLogEntry, SessionLogFilter, SessionLogKind};
use crate::file_lock::acquire_exclusive;

const LOG_FILE_NAME: &str = "session-log.jsonl";
const LOCK_FILE_NAME: &str = ".session-log.jsonl.lock";

pub(super) fn append(
    orbit_dir: &Path,
    params: SessionLogAppendParams,
) -> Result<SessionLogEntry, OrbitError> {
    let body = params.body.trim().to_string();
    if body.is_empty() {
        return Err(OrbitError::InvalidInput(
            "session-log body must be non-empty".to_string(),
        ));
    }

    let _guard = acquire_exclusive(&lock_path(orbit_dir), "session-log append")?;
    let path = log_path(orbit_dir);
    let entries = read_entries(&path)?;
    let entry = SessionLogEntry {
        id: next_id(&entries)?,
        at: Utc::now(),
        kind: params.kind,
        body,
        related_task_ids: params.related_task_ids,
        related_run_ids: params.related_run_ids,
        resolved_at: None,
    };
    append_entry(&path, &entry)?;
    Ok(entry)
}

pub(super) fn list(
    orbit_dir: &Path,
    filter: &SessionLogFilter,
) -> Result<Vec<SessionLogEntry>, OrbitError> {
    let _guard = acquire_exclusive(&lock_path(orbit_dir), "session-log read")?;
    let mut entries = read_entries(&log_path(orbit_dir))?;
    entries.retain(|entry| {
        filter.kind.is_none_or(|want| entry.kind == want)
            && filter.since.is_none_or(|bound| entry.at >= bound)
            && (!filter.unresolved_only
                || (entry.kind == SessionLogKind::CheckLater && entry.resolved_at.is_none()))
    });
    Ok(entries)
}

pub(super) fn resolve(orbit_dir: &Path, id: &str) -> Result<SessionLogEntry, OrbitError> {
    let id = id.trim();
    if id.is_empty() {
        return Err(OrbitError::InvalidInput(
            "session-log id is required".to_string(),
        ));
    }

    let _guard = acquire_exclusive(&lock_path(orbit_dir), "session-log resolve")?;
    let path = log_path(orbit_dir);
    let mut entries = read_entries(&path)?;
    let Some(index) = entries.iter().position(|entry| entry.id == id) else {
        return Err(OrbitError::InvalidInput(format!(
            "session-log id not found: {id}"
        )));
    };
    {
        let entry = &mut entries[index];
        if entry.kind != SessionLogKind::CheckLater {
            return Err(OrbitError::InvalidInput(format!(
                "{} is kind {:?} — only check_later entries can be resolved",
                entry.id, entry.kind
            )));
        }
        if entry.resolved_at.is_some() {
            return Err(OrbitError::InvalidInput(format!(
                "{} is already resolved",
                entry.id
            )));
        }
        entry.resolved_at = Some(Utc::now());
    }
    replace_entries(&path, &entries)?;
    Ok(entries[index].clone())
}

fn log_path(orbit_dir: &Path) -> PathBuf {
    orbit_dir.join(LOG_FILE_NAME)
}

fn lock_path(orbit_dir: &Path) -> PathBuf {
    orbit_dir.join(LOCK_FILE_NAME)
}

fn next_id(entries: &[SessionLogEntry]) -> Result<String, OrbitError> {
    let current = entries
        .iter()
        .filter_map(|entry| {
            entry
                .id
                .strip_prefix("SL-")
                .and_then(|digits| digits.parse::<u64>().ok())
        })
        .max()
        .unwrap_or(0);
    let next = current.checked_add(1).ok_or_else(|| {
        OrbitError::Store("session-log sequential ID space is exhausted".to_string())
    })?;
    Ok(format!("SL-{next:04}"))
}

fn append_entry(path: &Path, entry: &SessionLogEntry) -> Result<(), OrbitError> {
    let mut encoded = serde_json::to_vec(entry)
        .map_err(|error| OrbitError::Execution(format!("serialize session-log entry: {error}")))?;
    encoded.push(b'\n');

    let existed = path.exists();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| OrbitError::Io(format!("open {}: {error}", path.display())))?;
    file.write_all(&encoded)
        .map_err(|error| OrbitError::Io(format!("append {}: {error}", path.display())))?;
    file.sync_data()
        .map_err(|error| OrbitError::Io(format!("sync {}: {error}", path.display())))?;
    if !existed {
        sync_parent_dir(path).map_err(|error| {
            OrbitError::Io(format!("sync parent for {}: {error}", path.display()))
        })?;
    }
    Ok(())
}

fn read_entries(path: &Path) -> Result<Vec<SessionLogEntry>, OrbitError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
    let mut entries = Vec::new();
    for (line_number, line) in BufReader::new(file).lines().enumerate() {
        let line =
            line.map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
        if line.trim().is_empty() {
            continue;
        }
        let entry = serde_json::from_str(&line).map_err(|error| {
            OrbitError::Execution(format!(
                "parse {} line {}: {error}",
                path.display(),
                line_number + 1
            ))
        })?;
        entries.push(entry);
    }
    Ok(entries)
}

fn replace_entries(path: &Path, entries: &[SessionLogEntry]) -> Result<(), OrbitError> {
    let mut encoded = Vec::new();
    for entry in entries {
        serde_json::to_writer(&mut encoded, entry).map_err(|error| {
            OrbitError::Execution(format!("serialize session-log entry: {error}"))
        })?;
        encoded.push(b'\n');
    }
    atomic_write_bytes(path, &encoded)
        .map_err(|error| OrbitError::Io(format!("replace {}: {error}", path.display())))
}
