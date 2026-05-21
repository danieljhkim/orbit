use std::collections::BTreeMap;
use std::sync::Arc;

use orbit_common::types::{
    LearningInjectionCaps, LearningInjectionState, LearningReminder, OrbitError,
};
use orbit_common::utility::learning_session::{
    learning_session_state_path, read_learning_session_state, update_learning_session_state,
};
use orbit_common::utility::selector::anchor_path;
use rmcp::ErrorData as McpError;
use serde_json::{Value, json};

use super::{OrbitToolServer, PROCESS_LEARNING_SESSION_KEY};
use crate::McpHost;

impl OrbitToolServer {
    pub(super) async fn maybe_attach_learning_sidecar(
        &self,
        canonical: &str,
        input: Value,
        value: Value,
    ) -> Result<Value, McpError> {
        if !learning_sidecar_tool(canonical) {
            return Ok(value);
        }
        let paths = collect_learning_candidate_paths(&input, &value);
        if paths.is_empty() {
            return Ok(value);
        }

        let host = Arc::clone(&self.host);
        let caps = self.learning_caps;
        let join =
            tokio::task::spawn_blocking(move || search_learning_reminders(&*host, &paths, caps))
                .await;
        let reminders = match join {
            Ok(Ok(reminders)) => reminders,
            Ok(Err(error)) => {
                tracing::warn!(
                    target: "orbit.mcp.learnings",
                    error = %error,
                    "failed to search learning sidecar",
                );
                Vec::new()
            }
            Err(error) => {
                tracing::warn!(
                    target: "orbit.mcp.learnings",
                    error = %error,
                    "learning sidecar worker failed",
                );
                Vec::new()
            }
        };
        if reminders.is_empty() {
            return Ok(value);
        }

        let admitted = self.admit_learning_reminders(reminders).await?;
        Ok(attach_learning_sidecar(value, admitted))
    }

    async fn admit_learning_reminders(
        &self,
        reminders: Vec<LearningReminder>,
    ) -> Result<Vec<LearningReminder>, McpError> {
        let key = self.learning_session_key();
        let caps = self.learning_caps;
        if let Some(session_id) = self.learning_session_id.clone() {
            let root = std::env::current_dir().map_err(|error| {
                McpError::internal_error(
                    format!("resolve current dir for learning session: {error}"),
                    None,
                )
            })?;
            let path = learning_session_state_path(&root, &session_id);
            let reminders_for_file = reminders.clone();
            let join = tokio::task::spawn_blocking(move || {
                update_learning_session_state(&path, |state| {
                    state.admit_reminders(&reminders_for_file, caps)
                })
            })
            .await
            .map_err(|error| {
                McpError::internal_error(
                    format!("learning session state worker failed: {error}"),
                    None,
                )
            })?;
            let (state, admitted) = join.map_err(|error| {
                McpError::internal_error(format!("update learning session state: {error}"), None)
            })?;
            let mut states = self.learning_states.lock().await;
            states.insert(key, state);
            return Ok(admitted);
        }

        let mut states = self.learning_states.lock().await;
        let state = states.entry(key).or_default();
        Ok(state.admit_reminders(&reminders, caps))
    }

    fn learning_session_key(&self) -> String {
        self.learning_session_id
            .clone()
            .unwrap_or_else(|| PROCESS_LEARNING_SESSION_KEY.to_string())
    }
}

pub(super) fn load_learning_state_for_session(session_id: &str) -> Option<LearningInjectionState> {
    let root = std::env::current_dir().ok()?;
    let path = learning_session_state_path(&root, session_id);
    read_learning_session_state(&path).ok().flatten()
}

fn learning_sidecar_tool(canonical: &str) -> bool {
    matches!(
        canonical,
        "orbit.graph.show" | "orbit.graph.refs" | "orbit.task.show"
    )
}

fn collect_learning_candidate_paths(input: &Value, response: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    collect_paths_from_input(input, &mut paths);
    collect_paths_from_response(response, &mut paths);
    paths
}

fn collect_paths_from_input(value: &Value, out: &mut Vec<String>) {
    let Some(object) = value.as_object() else {
        return;
    };
    for key in ["selector", "selectors", "path", "paths"] {
        if let Some(value) = object.get(key) {
            collect_path_values(value, out);
        }
    }
}

fn collect_paths_from_response(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(key.as_str(), "file" | "path" | "context_files") {
                    collect_path_values(value, out);
                    continue;
                }
                if key == "code_refs" {
                    collect_code_ref_paths(value, out);
                    continue;
                }
                collect_paths_from_response(value, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_paths_from_response(item, out);
            }
        }
        _ => {}
    }
}

fn collect_code_ref_paths(value: &Value, out: &mut Vec<String>) {
    let Some(items) = value.as_array() else {
        return;
    };
    for item in items {
        if let Some(file) = item.get("file") {
            collect_path_values(file, out);
        }
    }
}

fn collect_path_values(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(raw) => push_candidate_path(raw, out),
        Value::Array(items) => {
            for item in items {
                collect_path_values(item, out);
            }
        }
        _ => {}
    }
}

fn push_candidate_path(raw: &str, out: &mut Vec<String>) {
    let Ok(path) = anchor_path(raw) else {
        return;
    };
    let path = path.to_string_lossy().replace('\\', "/");
    if !path.is_empty() && !out.iter().any(|existing| existing == &path) {
        out.push(path);
    }
}

#[derive(Debug, Clone)]
struct ReminderCandidate {
    reminder: LearningReminder,
    priority: Option<u8>,
    updated_at: String,
}

fn search_learning_reminders(
    host: &dyn McpHost,
    paths: &[String],
    caps: LearningInjectionCaps,
) -> Result<Vec<LearningReminder>, OrbitError> {
    // ORB-00202: per-domain `orbit.learning.search` was retired; the
    // applicability lookup re-homed onto `orbit.learning.list` with glob-
    // containment `path` semantics.
    let mut by_id: BTreeMap<String, ReminderCandidate> = BTreeMap::new();
    for path in paths {
        let value = host.call_tool(
            "orbit.learning.list",
            json!({
                "path": path,
            }),
        )?;
        for candidate in parse_learning_list_candidates(&value) {
            by_id
                .entry(candidate.reminder.id.clone())
                .or_insert(candidate);
        }
    }
    let mut candidates: Vec<_> = by_id.into_values().collect();
    candidates.sort_by(|a, b| {
        priority_rank(b.priority)
            .cmp(&priority_rank(a.priority))
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.reminder.id.cmp(&b.reminder.id))
    });
    candidates.truncate(caps.per_call);
    Ok(candidates
        .into_iter()
        .map(|candidate| candidate.reminder)
        .collect())
}

fn parse_learning_list_candidates(value: &Value) -> Vec<ReminderCandidate> {
    let items = value
        .as_array()
        .or_else(|| value.get("items").and_then(Value::as_array))
        .into_iter()
        .flatten();
    items
        .filter_map(|item| {
            let id = item.get("id").and_then(Value::as_str)?.to_string();
            let summary = item.get("summary").and_then(Value::as_str)?.to_string();
            let priority = item
                .get("priority")
                .and_then(Value::as_u64)
                .and_then(|value| u8::try_from(value).ok());
            let updated_at = item
                .get("updated_at")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(ReminderCandidate {
                reminder: LearningReminder {
                    id,
                    summary,
                    comments: Vec::new(),
                },
                priority,
                updated_at,
            })
        })
        .collect()
}

fn priority_rank(priority: Option<u8>) -> i16 {
    priority.map(i16::from).unwrap_or(-1)
}

fn attach_learning_sidecar(mut value: Value, reminders: Vec<LearningReminder>) -> Value {
    if reminders.is_empty() {
        return value;
    }
    let sidecar = Value::Array(
        reminders
            .into_iter()
            .map(|reminder| {
                json!({
                    "id": reminder.id,
                    "summary": reminder.summary,
                })
            })
            .collect(),
    );
    match &mut value {
        Value::Object(object) => {
            object.insert("learnings".to_string(), sidecar);
            value
        }
        _ => json!({
            "result": value,
            "learnings": sidecar,
        }),
    }
}

#[cfg(test)]
mod tests;
