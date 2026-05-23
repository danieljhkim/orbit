//! Task-review scoreboard auto-increment.
//!
//! Updates `.orbit/state/scoreboard/task_review.json` when local Orbit review
//! feedback is created:
//! - **review thread creation**: increment `task-review-threads`

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use orbit_common::types::{OrbitError, normalize_attribution_label};

use orbit_common::utility::fs::{
    atomic_write_text_volatile as write_atomic, with_exclusive_file_lock,
};

type ModelScores = HashMap<String, u64>;
type Scoreboard = HashMap<String, ModelScores>;

const TASK_REVIEW_THREADS_METRIC: &str = "task-review-threads";
const LEGACY_TASK_REVIEW_MESSAGES_METRIC: &str = "task-review-messages";

/// Increment the `task-review-threads` counter for the given model.
pub fn record_task_review_thread(scoreboard_dir: &Path, model: &str) -> Result<(), OrbitError> {
    increment(scoreboard_dir, TASK_REVIEW_THREADS_METRIC, model)
}

fn increment(scoreboard_dir: &Path, metric: &str, model: &str) -> Result<(), OrbitError> {
    let path = scoreboard_dir.join("task_review.json");
    let normalized_model = normalize_attribution_label(model, None);
    with_exclusive_file_lock(&path, "task review scoreboard", || {
        let mut scoreboard: Scoreboard = if path.exists() {
            let content = fs::read_to_string(&path)
                .map_err(|e| OrbitError::Io(format!("read task_review.json: {e}")))?;
            serde_json::from_str(&content)
                .map_err(|e| OrbitError::Io(format!("parse task_review.json: {e}")))?
        } else {
            HashMap::new()
        };

        migrate_legacy_messages_metric(&mut scoreboard);

        let model_map = scoreboard.entry(metric.to_string()).or_default();
        let counter = model_map.entry(normalized_model.clone()).or_insert(0);
        *counter += 1;

        let json = serde_json::to_string_pretty(&scoreboard)
            .map_err(|e| OrbitError::Io(format!("serialize task_review.json: {e}")))?;
        write_atomic(&path, &format!("{json}\n")).map_err(Into::into)
    })
}

fn migrate_legacy_messages_metric(scoreboard: &mut Scoreboard) {
    let Some(legacy_scores) = scoreboard.remove(LEGACY_TASK_REVIEW_MESSAGES_METRIC) else {
        return;
    };

    let thread_scores = scoreboard
        .entry(TASK_REVIEW_THREADS_METRIC.to_string())
        .or_default();
    for (model, count) in legacy_scores {
        let counter = thread_scores.entry(model).or_insert(0);
        *counter = counter.saturating_add(count);
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests;
