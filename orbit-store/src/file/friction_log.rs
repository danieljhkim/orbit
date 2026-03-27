use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use orbit_types::{FrictionEntry, OrbitError};

pub fn append_friction_entry(root: &Path, entry: &FrictionEntry) -> Result<(), OrbitError> {
    let file_path = friction_day_path(root, entry);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).map_err(|error| OrbitError::Io(error.to_string()))?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    let line =
        serde_json::to_string(entry).map_err(|error| OrbitError::Store(error.to_string()))?;
    writeln!(file, "{line}").map_err(|error| OrbitError::Io(error.to_string()))?;
    Ok(())
}

pub fn read_friction_entries_for_month(
    root: &Path,
    year_month: &str,
) -> Result<Vec<FrictionEntry>, OrbitError> {
    let month_dir = root
        .join("diagnostics")
        .join("friction")
        .join(validate_year_month(year_month)?);
    if !month_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = fs::read_dir(&month_dir)
        .map_err(|error| OrbitError::Io(error.to_string()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("jsonl"))
        .collect::<Vec<_>>();
    files.sort();

    let mut entries = Vec::new();
    for path in files {
        let raw = fs::read_to_string(&path).map_err(|error| OrbitError::Io(error.to_string()))?;
        for (index, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let entry = serde_json::from_str::<FrictionEntry>(line).map_err(|error| {
                OrbitError::Store(format!(
                    "invalid friction log entry at {}:{}: {error}",
                    path.display(),
                    index + 1
                ))
            })?;
            entries.push(entry);
        }
    }

    Ok(entries)
}

fn validate_year_month(raw: &str) -> Result<&str, OrbitError> {
    let bytes = raw.as_bytes();
    let valid = bytes.len() == 7
        && bytes[4] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..].iter().all(u8::is_ascii_digit);
    if valid {
        Ok(raw)
    } else {
        Err(OrbitError::InvalidInput(format!(
            "friction month must be in YYYY-MM format, got '{raw}'"
        )))
    }
}

fn friction_day_path(root: &Path, entry: &FrictionEntry) -> PathBuf {
    root.join("diagnostics")
        .join("friction")
        .join(entry.ts.format("%Y-%m").to_string())
        .join(format!("{}.jsonl", entry.ts.format("%d")))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use tempfile::tempdir;

    use super::{append_friction_entry, read_friction_entries_for_month};
    use orbit_types::{ActorIdentity, FrictionEntry};

    fn sample_entry(day: u32, command: &str) -> FrictionEntry {
        FrictionEntry {
            ts: Utc.with_ymd_and_hms(2026, 3, day, 12, 0, 0).unwrap(),
            job_run: format!("JR-{day}"),
            step: "review_pr".to_string(),
            task_id: Some(format!("T20260322-00000{day}")),
            command: command.to_string(),
            input: "{\"task_id\":\"T20260322-000001\"}".to_string(),
            exit_code: Some(1),
            stderr: "boom".to_string(),
            actor_identity: ActorIdentity::agent("codex", "gpt-5.4"),
        }
    }

    #[test]
    fn append_and_read_month_partitioned_friction_entries() {
        let dir = tempdir().expect("tempdir");
        append_friction_entry(dir.path(), &sample_entry(21, "orbit.task.show"))
            .expect("append entry");
        append_friction_entry(dir.path(), &sample_entry(22, "orbit.task.update"))
            .expect("append entry");

        let entries =
            read_friction_entries_for_month(dir.path(), "2026-03").expect("read friction entries");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "orbit.task.show");
        assert_eq!(entries[1].command, "orbit.task.update");
    }

    #[test]
    fn missing_month_returns_empty_entries() {
        let dir = tempdir().expect("tempdir");
        let entries =
            read_friction_entries_for_month(dir.path(), "2026-03").expect("read friction entries");
        assert!(entries.is_empty());
    }
}
