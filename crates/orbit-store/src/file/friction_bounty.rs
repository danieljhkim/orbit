//! Friction bounty scoreboard auto-increment.
//!
//! Updates `.orbit/state/scoreboard/friction_bounty.json` when self-reported friction tasks
//! transition through lifecycle states:
//! - **creation** (agent + model present): increment `issues-reported`
//! - **approval** (proposed→backlog, review→done): increment `issues-accepted`
//! - **rejection**: increment `issues-rejected`

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use orbit_types::{OrbitError, normalize_attribution_label};

use orbit_common::fs::{atomic_write_text_volatile as write_atomic, with_exclusive_file_lock};

type ModelScores = HashMap<String, u64>;
type Scoreboard = HashMap<String, ModelScores>;

/// Increment the `issues-reported` counter for the given model.
pub fn record_friction_reported(scoreboard_dir: &Path, model: &str) -> Result<(), OrbitError> {
    increment(scoreboard_dir, "issues-reported", model)
}

/// Increment the `issues-accepted` counter for the given model.
pub fn record_friction_accepted(scoreboard_dir: &Path, model: &str) -> Result<(), OrbitError> {
    increment(scoreboard_dir, "issues-accepted", model)
}

/// Increment the `issues-rejected` counter for the given model.
pub fn record_friction_rejected(scoreboard_dir: &Path, model: &str) -> Result<(), OrbitError> {
    increment(scoreboard_dir, "issues-rejected", model)
}

fn increment(scoreboard_dir: &Path, metric: &str, model: &str) -> Result<(), OrbitError> {
    let path = scoreboard_dir.join("friction_bounty.json");
    let normalized_model = normalize_attribution_label(model, None);
    with_exclusive_file_lock(&path, "friction bounty scoreboard", || {
        let mut scoreboard: Scoreboard = if path.exists() {
            let content = fs::read_to_string(&path)
                .map_err(|e| OrbitError::Io(format!("read friction_bounty.json: {e}")))?;
            serde_json::from_str(&content)
                .map_err(|e| OrbitError::Io(format!("parse friction_bounty.json: {e}")))?
        } else {
            HashMap::new()
        };

        let model_map = scoreboard.entry(metric.to_string()).or_default();
        let counter = model_map.entry(normalized_model.clone()).or_insert(0);
        *counter += 1;

        let json = serde_json::to_string_pretty(&scoreboard)
            .map_err(|e| OrbitError::Io(format!("serialize friction_bounty.json: {e}")))?;
        write_atomic(&path, &format!("{json}\n")).map_err(Into::into)
    })
}
