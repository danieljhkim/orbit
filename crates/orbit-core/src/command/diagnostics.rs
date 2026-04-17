//! Read access to per-step diagnostics (`metrics/`, `friction/` JSONL streams).
//!
//! Append paths are owned by the engine; this module exposes the symmetric
//! reader so command-layer surfaces (CLI, dashboard) can show recent entries
//! without bypassing `OrbitRuntime`.
//!
//! Tolerant of malformed JSONL lines: bad lines are skipped with a tracing
//! warning rather than failing the whole month. The on-disk logs are
//! append-only and have historically picked up partial writes from crashes;
//! a dashboard that crashes on one bad line is worse than one that omits it.

use std::fs;
use std::path::{Path, PathBuf};

use orbit_types::{FrictionEntry, MetricsEntry, OrbitError};
use serde::de::DeserializeOwned;

use crate::OrbitRuntime;

impl OrbitRuntime {
    pub fn read_metrics_entries(&self, year_month: &str) -> Result<Vec<MetricsEntry>, OrbitError> {
        read_jsonl_month::<MetricsEntry>(&self.data_root(), "metrics", year_month)
    }

    pub fn read_friction_entries(
        &self,
        year_month: &str,
    ) -> Result<Vec<FrictionEntry>, OrbitError> {
        read_jsonl_month::<FrictionEntry>(&self.data_root(), "friction", year_month)
    }
}

fn read_jsonl_month<T: DeserializeOwned>(
    root: &Path,
    category: &str,
    year_month: &str,
) -> Result<Vec<T>, OrbitError> {
    let month_dir: PathBuf = root
        .join("state")
        .join("diagnostics")
        .join(category)
        .join(year_month);
    if !month_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = fs::read_dir(&month_dir)
        .map_err(|e| OrbitError::Io(e.to_string()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|v| v.to_str()) == Some("jsonl"))
        .collect::<Vec<_>>();
    files.sort();

    let mut entries = Vec::new();
    for path in files {
        let raw = fs::read_to_string(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
        for (index, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<T>(line) {
                Ok(entry) => entries.push(entry),
                Err(err) => {
                    tracing::warn!(
                        target: "orbit::diagnostics",
                        path = %path.display(),
                        line = index + 1,
                        error = %err,
                        "skipping malformed diagnostics line"
                    );
                }
            }
        }
    }
    Ok(entries)
}
