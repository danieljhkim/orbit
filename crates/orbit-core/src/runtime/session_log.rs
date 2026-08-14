//! Workspace-scoped append-only session log [ADR-0363 / ORB-10784].
//!
//! Stored as JSONL at `<workspace>/.orbit/session-log.jsonl`. Bodies are never
//! rewritten; the only mutation is setting `resolved_at` on a `check_later`.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_common::types::OrbitError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const LOG_FILE_NAME: &str = "session-log.jsonl";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLogKind {
    Status,
    Note,
    CheckLater,
}

impl SessionLogKind {
    fn parse(raw: &str) -> Result<Self, OrbitError> {
        match raw.trim() {
            "status" => Ok(Self::Status),
            "note" => Ok(Self::Note),
            "check_later" => Ok(Self::CheckLater),
            other => Err(OrbitError::InvalidInput(format!(
                "unknown session-log kind `{other}` (expected status, note, or check_later)"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionLogEntry {
    pub id: String,
    pub at: DateTime<Utc>,
    pub kind: SessionLogKind,
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_task_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_run_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
}

pub fn log_path(orbit_dir: &Path) -> PathBuf {
    orbit_dir.join(LOG_FILE_NAME)
}

pub fn append(
    orbit_dir: &Path,
    kind: SessionLogKind,
    body: String,
    related_task_ids: Vec<String>,
    related_run_ids: Vec<String>,
) -> Result<SessionLogEntry, OrbitError> {
    let body = body.trim().to_string();
    if body.is_empty() {
        return Err(OrbitError::InvalidInput(
            "session-log body must be non-empty".to_string(),
        ));
    }
    fs::create_dir_all(orbit_dir).map_err(|err| {
        OrbitError::Io(format!(
            "create workspace orbit dir {}: {err}",
            orbit_dir.display()
        ))
    })?;
    let path = log_path(orbit_dir);
    let entries = read_entries(&path)?;
    let next_n = entries
        .iter()
        .filter_map(|entry| {
            entry
                .id
                .strip_prefix("SL-")
                .and_then(|digits| digits.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let entry = SessionLogEntry {
        id: format!("SL-{next_n:04}"),
        at: Utc::now(),
        kind,
        body,
        related_task_ids,
        related_run_ids,
        resolved_at: None,
    };
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| OrbitError::Io(format!("open {}: {err}", path.display())))?;
    writeln!(
        file,
        "{}",
        serde_json::to_string(&entry)
            .map_err(|err| OrbitError::Execution(format!("serialize session-log entry: {err}")))?
    )
    .map_err(|err| OrbitError::Io(format!("append {}: {err}", path.display())))?;
    Ok(entry)
}

pub fn list(
    orbit_dir: &Path,
    kind: Option<SessionLogKind>,
    unresolved_only: bool,
    since: Option<DateTime<Utc>>,
) -> Result<Vec<SessionLogEntry>, OrbitError> {
    let mut entries = read_entries(&log_path(orbit_dir))?;
    entries.retain(|entry| {
        kind.is_none_or(|want| entry.kind == want)
            && since.is_none_or(|bound| entry.at >= bound)
            && (!unresolved_only
                || (entry.kind == SessionLogKind::CheckLater && entry.resolved_at.is_none()))
    });
    Ok(entries)
}

pub fn resolve(orbit_dir: &Path, id: &str) -> Result<SessionLogEntry, OrbitError> {
    let id = id.trim();
    if id.is_empty() {
        return Err(OrbitError::InvalidInput(
            "session-log id is required".to_string(),
        ));
    }
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
    write_entries(&path, &entries)?;
    Ok(entries[index].clone())
}

fn read_entries(path: &Path) -> Result<Vec<SessionLogEntry>, OrbitError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path)
        .map_err(|err| OrbitError::Io(format!("read {}: {err}", path.display())))?;
    let mut entries = Vec::new();
    for (lineno, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|err| OrbitError::Io(format!("read {}: {err}", path.display())))?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: SessionLogEntry = serde_json::from_str(&line).map_err(|err| {
            OrbitError::Execution(format!(
                "parse {} line {}: {err}",
                path.display(),
                lineno + 1
            ))
        })?;
        entries.push(entry);
    }
    Ok(entries)
}

fn write_entries(path: &Path, entries: &[SessionLogEntry]) -> Result<(), OrbitError> {
    let tmp = path.with_extension("jsonl.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&tmp)
        .map_err(|err| OrbitError::Io(format!("write {}: {err}", tmp.display())))?;
    for entry in entries {
        writeln!(
            file,
            "{}",
            serde_json::to_string(entry).map_err(|err| OrbitError::Execution(format!(
                "serialize session-log entry: {err}"
            )))?
        )
        .map_err(|err| OrbitError::Io(format!("write {}: {err}", tmp.display())))?;
    }
    file.flush()
        .map_err(|err| OrbitError::Io(format!("flush {}: {err}", tmp.display())))?;
    fs::rename(&tmp, path).map_err(|err| {
        OrbitError::Io(format!(
            "replace {} with {}: {err}",
            path.display(),
            tmp.display()
        ))
    })?;
    Ok(())
}

pub(super) fn parse_kind(input: &Value) -> Result<SessionLogKind, OrbitError> {
    let raw = input
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| OrbitError::InvalidInput("kind is required".to_string()))?;
    SessionLogKind::parse(raw)
}

pub(super) fn parse_optional_kind(input: &Value) -> Result<Option<SessionLogKind>, OrbitError> {
    match input.get("kind") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => SessionLogKind::parse(raw).map(Some),
        Some(_) => Err(OrbitError::InvalidInput(
            "kind must be a string".to_string(),
        )),
    }
}

pub(super) fn parse_id_list(input: &Value, field: &str) -> Result<Vec<String>, OrbitError> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        OrbitError::InvalidInput(format!("{field} must be an array of strings"))
                    })
            })
            .collect(),
        Some(_) => Err(OrbitError::InvalidInput(format!(
            "{field} must be an array of strings"
        ))),
    }
}

pub(super) fn parse_since(input: &Value) -> Result<Option<DateTime<Utc>>, OrbitError> {
    match input.get("since") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => DateTime::parse_from_rfc3339(raw)
            .map(|dt| Some(dt.with_timezone(&Utc)))
            .map_err(|err| OrbitError::InvalidInput(format!("since must be RFC3339: {err}"))),
        Some(_) => Err(OrbitError::InvalidInput(
            "since must be an RFC3339 string".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_list_resolve_round_trip() {
        let root = tempdir().expect("tempdir");
        let orbit_dir = root.path().join(".orbit");

        let status = append(
            &orbit_dir,
            SessionLogKind::Status,
            "drained nothing".into(),
            vec![],
            vec![],
        )
        .expect("status");
        assert_eq!(status.id, "SL-0001");
        assert_eq!(status.kind, SessionLogKind::Status);

        let later = append(
            &orbit_dir,
            SessionLogKind::CheckLater,
            "recheck ORB-1 after CI".into(),
            vec!["ORB-1".into()],
            vec![],
        )
        .expect("check_later");
        assert_eq!(later.id, "SL-0002");
        assert!(later.resolved_at.is_none());

        let note = append(
            &orbit_dir,
            SessionLogKind::Note,
            "tycho owns the canary".into(),
            vec![],
            vec![],
        )
        .expect("note");
        assert_eq!(note.id, "SL-0003");

        let unresolved = list(&orbit_dir, None, true, None).expect("unresolved");
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].id, "SL-0002");

        let only_notes = list(&orbit_dir, Some(SessionLogKind::Note), false, None).expect("notes");
        assert_eq!(only_notes.len(), 1);
        assert_eq!(only_notes[0].id, "SL-0003");

        let resolved = resolve(&orbit_dir, "SL-0002").expect("resolve");
        assert!(resolved.resolved_at.is_some());
        assert!(list(&orbit_dir, None, true, None)
            .expect("after resolve")
            .is_empty());

        let err = resolve(&orbit_dir, "SL-0001").expect_err("status cannot resolve");
        assert!(err.to_string().contains("check_later"));
        let err = resolve(&orbit_dir, "SL-0002").expect_err("already resolved");
        assert!(err.to_string().contains("already resolved"));
    }
}
