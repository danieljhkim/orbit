use std::fs;
use std::path::Path;

use serde_json::json;

use orbit_types::OrbitError;

use crate::Store;

pub fn write_token_scoreboard(scoreboard_dir: &Path, store: &Store) -> Result<(), OrbitError> {
    let path = scoreboard_dir.join("tokens.json");
    let payload = json!({
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "activities": store.list_activity_invocation_metrics()?,
        "agents": store.list_agent_invocation_metrics()?,
        "top_tasks": store.list_top_task_invocation_metrics(20)?,
        "tools": store.list_tool_invocation_metrics()?,
        "known_limitations": [
            "Subagent attribution folds into the parent invocation totals.",
            "cache_read_tokens are reported separately from input_tokens.",
            "Multi-task invocations are fully attributed to every tagged task.",
            "Non-Claude providers currently emit zero traces."
        ]
    });

    fs::create_dir_all(scoreboard_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    let tmp = scoreboard_dir.join(".tokens.json.tmp");
    let raw = serde_json::to_string_pretty(&payload)
        .map_err(|e| OrbitError::Store(format!("serialize tokens.json: {e}")))?;
    fs::write(&tmp, format!("{raw}\n")).map_err(|e| OrbitError::Io(e.to_string()))?;
    fs::rename(&tmp, &path).map_err(|e| OrbitError::Io(e.to_string()))
}
