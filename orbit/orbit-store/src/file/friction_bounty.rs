//! Friction bounty scoreboard auto-increment.
//!
//! Updates `.orbit/scoreboard/friction_bounty.json` when friction/issue tasks
//! transition through lifecycle states:
//! - **creation** (agent + model present): increment `issues-reported`
//! - **approval** (proposed→backlog, review→done): increment `issues-accepted`
//! - **rejection**: increment `issues-rejected`

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use orbit_types::OrbitError;

type AgentScores = HashMap<String, HashMap<String, u64>>;
type Scoreboard = HashMap<String, AgentScores>;

/// Increment the `issues-reported` counter for the given agent/model.
pub fn record_friction_reported(
    scoreboard_dir: &Path,
    agent: &str,
    model: &str,
) -> Result<(), OrbitError> {
    increment(scoreboard_dir, "issues-reported", agent, model)
}

/// Increment the `issues-accepted` counter for the given agent/model.
pub fn record_friction_accepted(
    scoreboard_dir: &Path,
    agent: &str,
    model: &str,
) -> Result<(), OrbitError> {
    increment(scoreboard_dir, "issues-accepted", agent, model)
}

/// Increment the `issues-rejected` counter for the given agent/model.
pub fn record_friction_rejected(
    scoreboard_dir: &Path,
    agent: &str,
    model: &str,
) -> Result<(), OrbitError> {
    increment(scoreboard_dir, "issues-rejected", agent, model)
}

fn increment(
    scoreboard_dir: &Path,
    metric: &str,
    agent: &str,
    model: &str,
) -> Result<(), OrbitError> {
    let path = scoreboard_dir.join("friction_bounty.json");
    let mut scoreboard: Scoreboard = if path.exists() {
        let content = fs::read_to_string(&path)
            .map_err(|e| OrbitError::Io(format!("read friction_bounty.json: {e}")))?;
        serde_json::from_str(&content)
            .map_err(|e| OrbitError::Io(format!("parse friction_bounty.json: {e}")))?
    } else {
        HashMap::new()
    };

    let agent_map = scoreboard.entry(metric.to_string()).or_default();
    let model_map = agent_map.entry(agent.to_string()).or_default();
    let counter = model_map.entry(model.to_string()).or_insert(0);
    *counter += 1;

    let json = serde_json::to_string_pretty(&scoreboard)
        .map_err(|e| OrbitError::Io(format!("serialize friction_bounty.json: {e}")))?;

    // Write atomically via temp file + rename
    let dir = path
        .parent()
        .ok_or_else(|| OrbitError::Io("no parent dir for friction_bounty.json".to_string()))?;
    fs::create_dir_all(dir).map_err(|e| OrbitError::Io(format!("create scoreboard dir: {e}")))?;

    let tmp = dir.join(".friction_bounty.json.tmp");
    fs::write(&tmp, format!("{json}\n"))
        .map_err(|e| OrbitError::Io(format!("write friction_bounty tmp: {e}")))?;
    fs::rename(&tmp, &path)
        .map_err(|e| OrbitError::Io(format!("rename friction_bounty tmp: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{record_friction_accepted, record_friction_rejected, record_friction_reported};
    use std::fs;

    use serde_json::Value;
    use tempfile::tempdir;

    #[test]
    fn records_metrics_by_agent_and_model() {
        let dir = tempdir().expect("tempdir");

        record_friction_reported(dir.path(), "codex", "gpt-5.4").expect("reported");
        record_friction_reported(dir.path(), "codex", "gpt-5.4").expect("reported again");
        record_friction_accepted(dir.path(), "claude", "opus").expect("accepted");
        record_friction_rejected(dir.path(), "codex", "gpt-5.4").expect("rejected");

        let scoreboard: Value = serde_json::from_str(
            &fs::read_to_string(dir.path().join("friction_bounty.json")).expect("scoreboard"),
        )
        .expect("valid json");

        assert_eq!(scoreboard["issues-reported"]["codex"]["gpt-5.4"], 2);
        assert_eq!(scoreboard["issues-accepted"]["claude"]["opus"], 1);
        assert_eq!(scoreboard["issues-rejected"]["codex"]["gpt-5.4"], 1);
    }
}
