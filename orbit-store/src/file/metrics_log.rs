use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use orbit_types::{MetricsEntry, OrbitError};

pub fn append_metrics_entry(root: &Path, entry: &MetricsEntry) -> Result<(), OrbitError> {
    let file_path = metrics_day_path(root, entry);
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

pub fn read_metrics_entries_for_month(
    root: &Path,
    year_month: &str,
) -> Result<Vec<MetricsEntry>, OrbitError> {
    let month_dir = root
        .join("diagnostics")
        .join("metrics")
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
            let entry = serde_json::from_str::<MetricsEntry>(line).map_err(|error| {
                OrbitError::Store(format!(
                    "invalid metrics log entry at {}:{}: {error}",
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
            "metrics month must be in YYYY-MM format, got '{raw}'"
        )))
    }
}

fn metrics_day_path(root: &Path, entry: &MetricsEntry) -> PathBuf {
    root.join("diagnostics")
        .join("metrics")
        .join(entry.ts.format("%Y-%m").to_string())
        .join(format!("{}.jsonl", entry.ts.format("%d")))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use tempfile::tempdir;

    use super::{append_metrics_entry, read_metrics_entries_for_month};
    use orbit_types::{ActorIdentity, MetricsEntry};

    fn sample_entry(day: u32, invocations: u32) -> MetricsEntry {
        MetricsEntry {
            ts: Utc.with_ymd_and_hms(2026, 3, day, 12, 0, 0).unwrap(),
            job_run: format!("JR-{day}"),
            step: "execute_task".to_string(),
            task_id: Some(format!("T20260322-00000{day}")),
            actor_identity: ActorIdentity::agent("claude", "opus-4.6"),
            tool_invocations: invocations,
            token_usage: Some(15000),
            step_duration_ms: Some(45000),
            retry_count: 0,
        }
    }

    #[test]
    fn append_and_read_month_partitioned_metrics_entries() {
        let dir = tempdir().expect("tempdir");
        append_metrics_entry(dir.path(), &sample_entry(21, 8)).expect("append entry");
        append_metrics_entry(dir.path(), &sample_entry(22, 12)).expect("append entry");

        let entries =
            read_metrics_entries_for_month(dir.path(), "2026-03").expect("read metrics entries");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].tool_invocations, 8);
        assert_eq!(entries[1].tool_invocations, 12);
    }

    #[test]
    fn missing_month_returns_empty_entries() {
        let dir = tempdir().expect("tempdir");
        let entries =
            read_metrics_entries_for_month(dir.path(), "2026-03").expect("read metrics entries");
        assert!(entries.is_empty());
    }
}
