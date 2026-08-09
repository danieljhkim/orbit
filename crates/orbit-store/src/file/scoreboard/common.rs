//! Shared on-disk primitives for the `.orbit/state/scoreboard/` files.
//!
//! Per-model counter maps (`pr.json`, `task_review.json`) share a
//!   `{ "<metric>": { "<model>": <count> } }` map incremented under the same
//! lock. See [`increment_model_metric`].

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use orbit_common::types::{OrbitError, normalize_attribution_label};

use orbit_common::utility::fs::{
    atomic_write_text_volatile as write_atomic, with_exclusive_file_lock,
};

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
