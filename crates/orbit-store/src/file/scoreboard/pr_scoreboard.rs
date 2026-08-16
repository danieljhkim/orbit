//! PR scoreboard auto-increment.
//!
//! Updates `.orbit/state/scoreboard/pr.json` when PR lifecycle events occur:
//! - **merge without revision**: increment `pr-count-without-revision`
//! - **merge with revision**: increment `pr-count-with-revision`

use std::path::Path;

use orbit_common::OrbitError;

use super::common;

const SCOREBOARD_FILENAME: &str = "pr.json";
const LOCK_LABEL: &str = "pr scoreboard";

/// Increment the `pr-count-without-revision` counter for the given model.
pub fn record_pr_count_without_revision(
    scoreboard_dir: &Path,
    model: &str,
) -> Result<(), OrbitError> {
    increment(scoreboard_dir, "pr-count-without-revision", model)
}

/// Increment the `pr-count-with-revision` counter for the given model.
pub fn record_pr_count_with_revision(scoreboard_dir: &Path, model: &str) -> Result<(), OrbitError> {
    increment(scoreboard_dir, "pr-count-with-revision", model)
}

fn increment(scoreboard_dir: &Path, metric: &str, model: &str) -> Result<(), OrbitError> {
    common::increment_model_metric(
        scoreboard_dir,
        SCOREBOARD_FILENAME,
        LOCK_LABEL,
        metric,
        model,
        |_| {},
    )
}
