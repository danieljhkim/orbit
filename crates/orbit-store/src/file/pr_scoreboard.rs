//! PR scoreboard auto-increment.
//!
//! Updates `.orbit/state/scoreboard/pr.json` when PR lifecycle events occur:
//! - **review comment sync**: increment `pr-review-comments`
//! - **merge without revision**: increment `pr-count-without-revision`
//! - **merge with revision**: increment `pr-count-with-revision`

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use orbit_types::{OrbitError, normalize_attribution_label};

use orbit_common::fs::{atomic_write_text_volatile as write_atomic, with_exclusive_file_lock};

type ModelScores = HashMap<String, u64>;
type Scoreboard = HashMap<String, ModelScores>;

/// Increment the `pr-review-comments` counter for the given model.
pub fn record_pr_review_comment(scoreboard_dir: &Path, model: &str) -> Result<(), OrbitError> {
    increment(scoreboard_dir, "pr-review-comments", model)
}

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
    let path = scoreboard_dir.join("pr.json");
    let normalized_model = normalize_attribution_label(model, None);
    with_exclusive_file_lock(&path, "pr scoreboard", || {
        let mut scoreboard: Scoreboard = if path.exists() {
            let content = fs::read_to_string(&path)
                .map_err(|e| OrbitError::Io(format!("read pr.json: {e}")))?;
            serde_json::from_str(&content)
                .map_err(|e| OrbitError::Io(format!("parse pr.json: {e}")))?
        } else {
            HashMap::new()
        };

        let model_map = scoreboard.entry(metric.to_string()).or_default();
        let counter = model_map.entry(normalized_model.clone()).or_insert(0);
        *counter += 1;

        let json = serde_json::to_string_pretty(&scoreboard)
            .map_err(|e| OrbitError::Io(format!("serialize pr.json: {e}")))?;
        write_atomic(&path, &format!("{json}\n")).map_err(Into::into)
    })
}
