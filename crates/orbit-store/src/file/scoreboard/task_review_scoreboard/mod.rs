//! Task-review scoreboard auto-increment.
//!
//! Updates `.orbit/state/scoreboard/task_review.json` when local Orbit review
//! feedback is created:
//! - **review thread creation**: increment `task-review-threads`

use std::path::Path;

use orbit_common::types::OrbitError;

use super::common;
use super::common::{
    CounterScoreboard, LEGACY_TASK_REVIEW_MESSAGES_METRIC, TASK_REVIEW_THREADS_METRIC,
};

const SCOREBOARD_FILENAME: &str = "task_review.json";
const LOCK_LABEL: &str = "task review scoreboard";

/// Increment the `task-review-threads` counter for the given model.
pub fn record_task_review_thread(scoreboard_dir: &Path, model: &str) -> Result<(), OrbitError> {
    common::increment_model_metric(
        scoreboard_dir,
        SCOREBOARD_FILENAME,
        LOCK_LABEL,
        TASK_REVIEW_THREADS_METRIC,
        model,
        migrate_legacy_messages_metric,
    )
}

fn migrate_legacy_messages_metric(scoreboard: &mut CounterScoreboard) {
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
mod tests;
