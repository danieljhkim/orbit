//! Shared on-disk primitives for the `.orbit/state/scoreboard/` files.
//!
//! Two file shapes recur across the scoreboard modules and are consolidated
//! here so each scoreboard owns only its filename, lock label, and domain
//! types:
//! - **Append-only run logs** (`duel.json`, `duel_plan.json`) — a
//!   `{ "schema_version": 1, "runs": [...] }` envelope rewritten atomically
//!   under an exclusive lock. See [`append_run_entry`] / [`load_run_entries`].
//! - **Per-model counter maps** (`pr.json`) — a
//!   `{ "<metric>": { "<model>": <count> } }` map incremented under the same
//!   lock. See [`increment_model_metric`].

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use orbit_common::types::{OrbitError, normalize_attribution_label};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use orbit_common::utility::fs::{
    atomic_write_text_volatile as write_atomic, with_exclusive_file_lock,
};

/// Schema version stamped into newly created run-log envelopes.
const CURRENT_RUN_LOG_SCHEMA_VERSION: u32 = 1;

/// On-disk envelope shared by the append-only run-log scoreboards.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct RunLogFile<T> {
    schema_version: u32,
    // `default = "Vec::new"` (not bare `default`) so the serde derive does
    // not infer an unnecessary `T: Default` bound; behavior is identical.
    #[serde(default = "Vec::new")]
    runs: Vec<T>,
}

impl<T> Default for RunLogFile<T> {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_RUN_LOG_SCHEMA_VERSION,
            runs: Vec::new(),
        }
    }
}

/// Append one run entry to `scoreboard_dir/<file_name>`, creating the file on
/// first use. Uses the shared atomic-write helper under an exclusive lock so
/// a crash during the rewrite cannot corrupt earlier entries.
pub(crate) fn append_run_entry<T>(
    scoreboard_dir: &Path,
    file_name: &str,
    lock_label: &str,
    run: &T,
) -> Result<(), OrbitError>
where
    T: Serialize + DeserializeOwned + Clone,
{
    let path = scoreboard_dir.join(file_name);
    with_exclusive_file_lock(&path, lock_label, || {
        let mut file = load_run_log_file::<T>(&path, file_name)?;
        file.runs.push(run.clone());

        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| OrbitError::Io(format!("serialize {file_name}: {e}")))?;
        write_atomic(&path, &format!("{json}\n")).map_err(Into::into)
    })
}

/// Load every run entry from `scoreboard_dir/<file_name>`. Returns an empty
/// vector when the file does not yet exist (or is empty).
pub(crate) fn load_run_entries<T>(
    scoreboard_dir: &Path,
    file_name: &str,
) -> Result<Vec<T>, OrbitError>
where
    T: Serialize + DeserializeOwned,
{
    let path = scoreboard_dir.join(file_name);
    Ok(load_run_log_file::<T>(&path, file_name)?.runs)
}

fn load_run_log_file<T>(path: &Path, file_name: &str) -> Result<RunLogFile<T>, OrbitError>
where
    T: Serialize + DeserializeOwned,
{
    if !path.exists() {
        return Ok(RunLogFile::default());
    }
    let content =
        fs::read_to_string(path).map_err(|e| OrbitError::Io(format!("read {file_name}: {e}")))?;
    if content.trim().is_empty() {
        return Ok(RunLogFile::default());
    }
    serde_json::from_str(&content).map_err(|e| OrbitError::Io(format!("parse {file_name}: {e}")))
}

/// Per-model counters for one metric.
pub(crate) type ModelScores = HashMap<String, u64>;
/// Metric name → per-model counters — the shape of `pr.json` and
/// `task_review.json`.
pub(crate) type CounterScoreboard = HashMap<String, ModelScores>;

/// Increment `metric` for the (normalized) `model` in
/// `scoreboard_dir/<file_name>`, creating the file on first use. `migrate`
/// runs on the freshly-loaded scoreboard before the increment so callers can
/// fold legacy metric keys into their canonical form.
pub(crate) fn increment_model_metric(
    scoreboard_dir: &Path,
    file_name: &str,
    lock_label: &str,
    metric: &str,
    model: &str,
    migrate: impl FnOnce(&mut CounterScoreboard),
) -> Result<(), OrbitError> {
    let path = scoreboard_dir.join(file_name);
    let normalized_model = normalize_attribution_label(model, None);
    with_exclusive_file_lock(&path, lock_label, || {
        let mut scoreboard: CounterScoreboard = if path.exists() {
            let content = fs::read_to_string(&path)
                .map_err(|e| OrbitError::Io(format!("read {file_name}: {e}")))?;
            serde_json::from_str(&content)
                .map_err(|e| OrbitError::Io(format!("parse {file_name}: {e}")))?
        } else {
            HashMap::new()
        };

        migrate(&mut scoreboard);

        let model_map = scoreboard.entry(metric.to_string()).or_default();
        let counter = model_map.entry(normalized_model.clone()).or_insert(0);
        *counter += 1;

        let json = serde_json::to_string_pretty(&scoreboard)
            .map_err(|e| OrbitError::Io(format!("serialize {file_name}: {e}")))?;
        write_atomic(&path, &format!("{json}\n")).map_err(Into::into)
    })
}
