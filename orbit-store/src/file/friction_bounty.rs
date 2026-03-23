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
    repo_root: &Path,
    agent: &str,
    model: &str,
) -> Result<(), OrbitError> {
    increment(repo_root, "issues-reported", agent, model)
}

/// Increment the `issues-accepted` counter for the given agent/model.
pub fn record_friction_accepted(
    repo_root: &Path,
    agent: &str,
    model: &str,
) -> Result<(), OrbitError> {
    increment(repo_root, "issues-accepted", agent, model)
}

/// Increment the `issues-rejected` counter for the given agent/model.
pub fn record_friction_rejected(
    repo_root: &Path,
    agent: &str,
    model: &str,
) -> Result<(), OrbitError> {
    increment(repo_root, "issues-rejected", agent, model)
}

fn increment(repo_root: &Path, metric: &str, agent: &str, model: &str) -> Result<(), OrbitError> {
    let path = repo_root
        .join(".orbit")
        .join("scoreboard")
        .join("friction_bounty.json");
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
    use super::*;

    #[test]
    fn increment_creates_and_updates_scoreboard() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // First increment creates the file
        record_friction_reported(root, "claude", "opus-4.6").unwrap();

        let path = root.join(".orbit/scoreboard/friction_bounty.json");
        assert!(path.exists());
        let sb: Scoreboard = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(sb["issues-reported"]["claude"]["opus-4.6"], 1);

        // Second increment bumps the counter
        record_friction_reported(root, "claude", "opus-4.6").unwrap();
        let sb: Scoreboard = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(sb["issues-reported"]["claude"]["opus-4.6"], 2);

        // Different metric
        record_friction_accepted(root, "claude", "opus-4.6").unwrap();
        let sb: Scoreboard = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(sb["issues-accepted"]["claude"]["opus-4.6"], 1);
        // Original metric unchanged
        assert_eq!(sb["issues-reported"]["claude"]["opus-4.6"], 2);
    }

    #[test]
    fn increment_different_agents() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        record_friction_reported(root, "claude", "opus-4.6").unwrap();
        record_friction_reported(root, "codex", "gpt-5.4").unwrap();

        let path = root.join(".orbit/scoreboard/friction_bounty.json");
        let sb: Scoreboard = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(sb["issues-reported"]["claude"]["opus-4.6"], 1);
        assert_eq!(sb["issues-reported"]["codex"]["gpt-5.4"], 1);
    }
}
