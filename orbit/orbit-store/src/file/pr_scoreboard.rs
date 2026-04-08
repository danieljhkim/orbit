//! PR scoreboard auto-increment.
//!
//! Updates `.orbit/scoreboard/pr.json` when PR lifecycle events occur:
//! - **review comment sync**: increment `pr-review-comments`
//! - **merge without revision**: increment `pr-count-without-revision`
//! - **merge with revision**: increment `pr-count-with-revision`

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use orbit_types::OrbitError;

type AgentScores = HashMap<String, HashMap<String, u64>>;
type Scoreboard = HashMap<String, AgentScores>;

/// Increment the `pr-review-comments` counter for the given agent/model.
pub fn record_pr_review_comment(
    scoreboard_dir: &Path,
    agent: &str,
    model: &str,
) -> Result<(), OrbitError> {
    increment(scoreboard_dir, "pr-review-comments", agent, model)
}

/// Increment the `pr-count-without-revision` counter for the given agent/model.
pub fn record_pr_count_without_revision(
    scoreboard_dir: &Path,
    agent: &str,
    model: &str,
) -> Result<(), OrbitError> {
    increment(scoreboard_dir, "pr-count-without-revision", agent, model)
}

/// Increment the `pr-count-with-revision` counter for the given agent/model.
pub fn record_pr_count_with_revision(
    scoreboard_dir: &Path,
    agent: &str,
    model: &str,
) -> Result<(), OrbitError> {
    increment(scoreboard_dir, "pr-count-with-revision", agent, model)
}

fn increment(
    scoreboard_dir: &Path,
    metric: &str,
    agent: &str,
    model: &str,
) -> Result<(), OrbitError> {
    let path = scoreboard_dir.join("pr.json");
    let mut scoreboard: Scoreboard = if path.exists() {
        let content =
            fs::read_to_string(&path).map_err(|e| OrbitError::Io(format!("read pr.json: {e}")))?;
        serde_json::from_str(&content).map_err(|e| OrbitError::Io(format!("parse pr.json: {e}")))?
    } else {
        HashMap::new()
    };

    let agent_map = scoreboard.entry(metric.to_string()).or_default();
    let model_map = agent_map.entry(agent.to_string()).or_default();
    let counter = model_map.entry(model.to_string()).or_insert(0);
    *counter += 1;

    let json = serde_json::to_string_pretty(&scoreboard)
        .map_err(|e| OrbitError::Io(format!("serialize pr.json: {e}")))?;

    // Write atomically via temp file + rename
    let dir = path
        .parent()
        .ok_or_else(|| OrbitError::Io("no parent dir for pr.json".to_string()))?;
    fs::create_dir_all(dir).map_err(|e| OrbitError::Io(format!("create scoreboard dir: {e}")))?;

    let tmp = dir.join(".pr.json.tmp");
    fs::write(&tmp, format!("{json}\n"))
        .map_err(|e| OrbitError::Io(format!("write pr.json tmp: {e}")))?;
    fs::rename(&tmp, &path).map_err(|e| OrbitError::Io(format!("rename pr.json tmp: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::Value;
    use tempfile::tempdir;

    use super::{
        record_pr_count_with_revision, record_pr_count_without_revision, record_pr_review_comment,
    };

    #[test]
    fn records_simplified_pr_metrics_by_agent_and_model() {
        let dir = tempdir().expect("tempdir");

        record_pr_review_comment(dir.path(), "claude", "sonnet").expect("review comment");
        record_pr_review_comment(dir.path(), "claude", "sonnet").expect("review comment again");
        record_pr_count_without_revision(dir.path(), "codex", "gpt-5.4")
            .expect("merged without revision");
        record_pr_count_with_revision(dir.path(), "codex", "gpt-5.4")
            .expect("merged with revision");

        let scoreboard: Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("pr.json")).expect("pr.json"))
                .expect("valid json");

        assert_eq!(scoreboard["pr-review-comments"]["claude"]["sonnet"], 2);
        assert_eq!(
            scoreboard["pr-count-without-revision"]["codex"]["gpt-5.4"],
            1
        );
        assert_eq!(scoreboard["pr-count-with-revision"]["codex"]["gpt-5.4"], 1);
        assert!(scoreboard.get("prs-merged").is_none());
        assert!(scoreboard.get("revisions").is_none());
        assert!(scoreboard.get("comments-resolved").is_none());
    }
}
