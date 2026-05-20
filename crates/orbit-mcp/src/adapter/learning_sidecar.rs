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
    let mut by_id: BTreeMap<String, ReminderCandidate> = BTreeMap::new();
    for path in paths {
        let value = host.call_tool(
            "orbit.learning.search",
            json!({
                "path": path,
                "limit": caps.per_call,
            }),
        )?;
        for candidate in parse_learning_search_candidates(&value) {
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

fn parse_learning_search_candidates(value: &Value) -> Vec<ReminderCandidate> {
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
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use orbit_common::types::{LearningInjectionCaps, LearningInjectionState};
    use serde_json::{Value, json};

    use super::super::test_support::{LearningSidecarHost, request_with_args};
    use super::super::{OrbitToolServer, PROCESS_LEARNING_SESSION_KEY};

    #[tokio::test]
    async fn learning_sidecar_present_with_summary_only_on_path_match() {
        let mut search_by_path = HashMap::new();
        search_by_path.insert(
            "crates/orbit-engine/src/lib.rs".to_string(),
            vec![json!({
                "id": "L-0001",
                "summary": "Remember the engine rule.",
                "body": "full body must stay out",
                "updated_at": "2026-05-15T00:00:00Z",
                "priority": 7
            })],
        );
        let host = Arc::new(LearningSidecarHost::new(
            json!({
                "code_refs": [{"file": "crates/orbit-engine/src/lib.rs"}]
            }),
            search_by_path,
        ));
        let server = OrbitToolServer::new_for_test(
            host,
            None,
            LearningInjectionCaps::default(),
            LearningInjectionState::default(),
        );

        let result = server
            .call_tool_request(request_with_args(
                "orbit.graph.show",
                json!({"selector": "file:crates/orbit-engine/src/lib.rs"}),
            ))
            .await
            .expect("call succeeds");
        let structured = result
            .structured_content
            .as_ref()
            .expect("structured content");

        assert_eq!(
            structured.get("learnings"),
            Some(&json!([{
                "id": "L-0001",
                "summary": "Remember the engine rule."
            }]))
        );
        assert!(
            !serde_json::to_string(structured)
                .expect("json")
                .contains("full body")
        );
    }

    #[tokio::test]
    async fn learning_sidecar_absent_when_no_learning_matches() {
        let mut search_by_path = HashMap::new();
        search_by_path.insert("crates/orbit-engine/src/lib.rs".to_string(), Vec::new());
        let host = Arc::new(LearningSidecarHost::new(
            json!({
                "code_refs": [{"file": "crates/orbit-engine/src/lib.rs"}]
            }),
            search_by_path,
        ));
        let server = OrbitToolServer::new_for_test(
            host,
            None,
            LearningInjectionCaps::default(),
            LearningInjectionState::default(),
        );

        let result = server
            .call_tool_request(request_with_args(
                "orbit.graph.refs",
                json!({"selector": "file:crates/orbit-engine/src/lib.rs"}),
            ))
            .await
            .expect("call succeeds");
        let structured = result
            .structured_content
            .as_ref()
            .expect("structured content");

        assert!(structured.get("learnings").is_none());
    }

    #[tokio::test]
    async fn l1_seeded_learning_is_suppressed_by_l2_dedup_state() {
        let mut search_by_path = HashMap::new();
        search_by_path.insert(
            "crates/orbit-engine/src/lib.rs".to_string(),
            vec![json!({
                "id": "L-0001",
                "summary": "Already injected at L1.",
                "updated_at": "2026-05-15T00:00:00Z",
                "priority": null
            })],
        );
        let host = Arc::new(LearningSidecarHost::new(
            json!({
                "context_files": ["file:crates/orbit-engine/src/lib.rs"]
            }),
            search_by_path,
        ));
        let initial_state = LearningInjectionState::seeded(["L-0001".to_string()]);
        let server = OrbitToolServer::new_for_test(
            host,
            None,
            LearningInjectionCaps::default(),
            initial_state,
        );

        let result = server
            .call_tool_request(request_with_args(
                "orbit.task.show",
                json!({"id": "ORB-00009"}),
            ))
            .await
            .expect("call succeeds");
        let structured = result
            .structured_content
            .as_ref()
            .expect("structured content");

        assert!(structured.get("learnings").is_none());
        let states = server.learning_states.lock().await;
        let state = states.get(PROCESS_LEARNING_SESSION_KEY).expect("state");
        assert_eq!(state.count, 1);
        assert!(state.emitted_ids.contains("L-0001"));
    }

    #[tokio::test]
    async fn learning_sidecar_enforces_per_session_hard_cap() {
        let mut search_by_path = HashMap::new();
        for call_idx in 0..5 {
            let path = format!("p{call_idx}.rs");
            let rows: Vec<_> = (0..5)
                .map(|row_idx| {
                    let id_idx = call_idx * 5 + row_idx;
                    json!({
                        "id": format!("L-{id_idx:04}"),
                        "summary": format!("Learning {id_idx}"),
                        "updated_at": "2026-05-15T00:00:00Z",
                        "priority": null
                    })
                })
                .collect();
            search_by_path.insert(path, rows);
        }
        let host = Arc::new(LearningSidecarHost::new(json!({}), search_by_path));
        let server = OrbitToolServer::new_for_test(
            host,
            None,
            LearningInjectionCaps {
                per_call: 5,
                per_session_hard: 20,
            },
            LearningInjectionState::default(),
        );
        let mut emitted = 0usize;

        for call_idx in 0..5 {
            let result = server
                .call_tool_request(request_with_args(
                    "orbit.graph.show",
                    json!({"selector": format!("file:p{call_idx}.rs")}),
                ))
                .await
                .expect("call succeeds");
            let structured = result
                .structured_content
                .as_ref()
                .expect("structured content");
            emitted += structured
                .get("learnings")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default();
        }

        assert_eq!(emitted, 20);
        let states = server.learning_states.lock().await;
        let state = states.get(PROCESS_LEARNING_SESSION_KEY).expect("state");
        assert_eq!(state.count, 20);
        assert_eq!(state.emitted_ids.len(), 20);
    }
}
