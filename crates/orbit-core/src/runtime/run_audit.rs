use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use orbit_common::types::OrbitError;
use orbit_common::utility::blob_store::BlobStore;
use serde_json::Value;

use crate::OrbitRuntime;

#[derive(Clone, Debug, PartialEq)]
pub struct RunAuditEvent {
    pub raw: Value,
    pub event_id: String,
    pub parent_event_id: Option<String>,
    pub event_type: Option<String>,
    pub body_kind: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub step_id: Option<String>,
}

impl RunAuditEvent {
    pub fn json_with_step_id(&self) -> Value {
        let mut raw = self.raw.clone();
        if let Some(step_id) = &self.step_id
            && raw.get("step_id").is_none()
            && let Some(object) = raw.as_object_mut()
        {
            object.insert("step_id".to_string(), Value::String(step_id.clone()));
        }
        raw
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunAuditStep {
    pub step_index: u32,
    pub step_id: String,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub state: Option<String>,
    pub outcome: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunCliInvocationRecord {
    pub run_id: String,
    pub event_id: String,
    pub ts: Option<DateTime<Utc>>,
    pub step_id: Option<String>,
    pub step_index: Option<u32>,
    pub provider: Option<String>,
    pub stdout_blob_ref: Option<String>,
    pub stderr_blob_ref: Option<String>,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i64>,
    pub timed_out: bool,
    pub duration_ms: Option<u64>,
}

impl OrbitRuntime {
    pub fn collect_run_audit_events(&self, run_id: &str) -> Result<Vec<RunAuditEvent>, OrbitError> {
        let audit_path = self.v2_audit_log_path(run_id);
        if !audit_path.exists() {
            return Ok(Vec::new());
        }

        let raw = fs::read_to_string(&audit_path).map_err(|err| {
            OrbitError::Io(format!("read audit log '{}': {err}", audit_path.display()))
        })?;
        let mut events_by_id = HashMap::new();
        let mut ordered_ids = Vec::new();
        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let value: Value = match serde_json::from_str(line) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let Some(event_id) = value.get("event_id").and_then(Value::as_str) else {
                continue;
            };
            ordered_ids.push(event_id.to_string());
            events_by_id.insert(event_id.to_string(), value);
        }

        let mut events = Vec::new();
        for event_id in ordered_ids {
            let Some(raw) = events_by_id.get(&event_id).cloned() else {
                continue;
            };
            events.push(RunAuditEvent {
                parent_event_id: raw
                    .get("parent_event_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                event_type: raw
                    .get("event_type")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                body_kind: raw
                    .get("body_kind")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                timestamp: raw
                    .get("ts")
                    .and_then(Value::as_str)
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.with_timezone(&Utc)),
                step_id: enclosing_step_id(&raw, &events_by_id),
                raw,
                event_id,
            });
        }

        Ok(events)
    }

    pub fn collect_run_audit_steps(&self, run_id: &str) -> Result<Vec<RunAuditStep>, OrbitError> {
        let events = self.collect_run_audit_events(run_id)?;
        let mut steps = Vec::<RunAuditStep>::new();
        let mut index_by_id = HashMap::<String, usize>::new();

        for event in events {
            match event.body_kind.as_deref() {
                Some("step_started") => {
                    let Some(step_id) = event.raw.get("step_id").and_then(Value::as_str) else {
                        continue;
                    };
                    if index_by_id.contains_key(step_id) {
                        continue;
                    }
                    let index = steps.len();
                    index_by_id.insert(step_id.to_string(), index);
                    steps.push(RunAuditStep {
                        step_index: index as u32,
                        step_id: step_id.to_string(),
                        started_at: event.timestamp,
                        finished_at: None,
                        state: None,
                        outcome: None,
                        error_message: None,
                    });
                }
                Some("step_finished") | Some("step_skipped") | Some("step_denied") => {
                    let Some(step_id) = event.raw.get("step_id").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(index) = index_by_id.get(step_id).copied() else {
                        continue;
                    };
                    let step = &mut steps[index];
                    step.finished_at = event.timestamp;
                    match event.body_kind.as_deref() {
                        Some("step_finished") => {
                            let outcome = event
                                .raw
                                .get("outcome")
                                .and_then(Value::as_str)
                                .unwrap_or("finished")
                                .to_string();
                            step.state = Some(outcome.clone());
                            step.outcome = Some(outcome);
                        }
                        Some("step_skipped") => {
                            step.state = Some("skipped".to_string());
                            step.outcome = Some("skipped".to_string());
                            step.error_message = event
                                .raw
                                .get("reason")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                        }
                        Some("step_denied") => {
                            step.state = Some("failed".to_string());
                            step.outcome = Some("denied".to_string());
                            step.error_message = event
                                .raw
                                .get("reason")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        Ok(steps)
    }

    pub fn collect_run_cli_invocations(
        &self,
        run_id: &str,
    ) -> Result<Vec<RunCliInvocationRecord>, OrbitError> {
        let events = self.collect_run_audit_events(run_id)?;
        let blob_store = BlobStore::new(self.v2_audit_blob_root());
        let step_index_by_id = self
            .collect_run_audit_steps(run_id)?
            .into_iter()
            .map(|step| (step.step_id, step.step_index))
            .collect::<HashMap<_, _>>();
        let mut records = Vec::new();

        for event in events {
            if event.body_kind.as_deref() != Some("cli_invocation_finished") {
                continue;
            }
            let stdout_blob_ref = event
                .raw
                .get("stdout_blob_ref")
                .and_then(Value::as_str)
                .map(str::to_string);
            let stderr_blob_ref = event
                .raw
                .get("stderr_blob_ref")
                .and_then(Value::as_str)
                .map(str::to_string);
            let stdout = match stdout_blob_ref.as_deref() {
                Some(blob_ref) => read_blob_text_best_effort(&blob_store, blob_ref),
                None => String::new(),
            };
            let stderr = match stderr_blob_ref.as_deref() {
                Some(blob_ref) => read_blob_text_best_effort(&blob_store, blob_ref),
                None => String::new(),
            };
            let step_index = event
                .step_id
                .as_ref()
                .and_then(|step_id| step_index_by_id.get(step_id).copied());
            records.push(RunCliInvocationRecord {
                run_id: event
                    .raw
                    .get("run_id")
                    .and_then(Value::as_str)
                    .unwrap_or(run_id)
                    .to_string(),
                event_id: event.event_id,
                ts: event.timestamp,
                step_index,
                step_id: event.step_id,
                provider: event
                    .raw
                    .get("provider")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                stdout_blob_ref,
                stderr_blob_ref,
                stdout,
                stderr,
                exit_code: event.raw.get("exit_code").and_then(Value::as_i64),
                timed_out: event
                    .raw
                    .get("timed_out")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                duration_ms: event.raw.get("duration_ms").and_then(Value::as_u64),
            });
        }

        Ok(records)
    }

    fn v2_audit_log_path(&self, run_id: &str) -> PathBuf {
        self.data_root()
            .join("state")
            .join("audit")
            .join("v2_loop")
            .join(format!("{run_id}.jsonl"))
    }

    fn v2_audit_blob_root(&self) -> PathBuf {
        self.data_root().join("state").join("audit").join("blobs")
    }
}

fn enclosing_step_id(event: &Value, events: &HashMap<String, Value>) -> Option<String> {
    if let Some(step_id) = event.get("step_id").and_then(Value::as_str) {
        return Some(step_id.to_string());
    }

    let mut parent_id = event
        .get("parent_event_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut seen = HashSet::new();
    while let Some(id) = parent_id {
        if !seen.insert(id.clone()) {
            return None;
        }
        let parent = events.get(&id)?;
        if parent.get("body_kind").and_then(Value::as_str) == Some("step_started") {
            return parent
                .get("step_id")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        parent_id = parent
            .get("parent_event_id")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    None
}

fn read_blob_text(blob_store: &BlobStore, blob_ref: &str) -> Result<String, OrbitError> {
    if blob_ref.len() < 2 || blob_ref.starts_with("error:") {
        return Err(OrbitError::Store(format!(
            "invalid audit blob reference '{blob_ref}'"
        )));
    }
    let bytes = blob_store
        .read(blob_ref)
        .map_err(|err| OrbitError::Io(format!("read audit blob '{blob_ref}': {err}")))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_blob_text_best_effort(blob_store: &BlobStore, blob_ref: &str) -> String {
    read_blob_text(blob_store, blob_ref).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    #[test]
    fn collect_run_cli_invocations_derives_step_ids_from_parent_chain() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let audit_root = runtime.data_root().join("state").join("audit");
        let blob_store = BlobStore::new(audit_root.join("blobs"));
        let stdout_one = blob_store.write(b"one stdout\n").expect("write stdout one");
        let stderr_one = blob_store.write(b"one stderr\n").expect("write stderr one");
        let stdout_two = blob_store.write(b"two stdout\n").expect("write stdout two");

        let jsonl_dir = audit_root.join("v2_loop");
        std::fs::create_dir_all(&jsonl_dir).expect("create jsonl dir");
        let run_id = "jrun-test";
        let events = [
            json!({
                "schemaVersion": 1,
                "event_type": "run.started",
                "event_id": "evt-run-started",
                "ts": "2026-04-26T07:00:00Z",
                "run_id": run_id,
                "agent_identity": "codex",
                "body_kind": "run_started",
                "job_name": "test-job"
            }),
            json!({
                "schemaVersion": 1,
                "event_type": "step.started",
                "event_id": "evt-step-one",
                "ts": "2026-04-26T07:00:01Z",
                "run_id": run_id,
                "agent_identity": "codex",
                "parent_event_id": "evt-run-started",
                "body_kind": "step_started",
                "step_id": "implement_one"
            }),
            json!({
                "schemaVersion": 1,
                "event_type": "activity.started",
                "event_id": "evt-activity-one",
                "ts": "2026-04-26T07:00:02Z",
                "run_id": run_id,
                "agent_identity": "codex",
                "parent_event_id": "evt-step-one",
                "body_kind": "activity_started",
                "activity_name": "worker",
                "activity_type": "agent_loop"
            }),
            json!({
                "schemaVersion": 1,
                "event_type": "cli.invocation.finished",
                "event_id": "evt-cli-one",
                "ts": "2026-04-26T07:00:03Z",
                "run_id": run_id,
                "agent_identity": "codex",
                "parent_event_id": "evt-activity-one",
                "body_kind": "cli_invocation_finished",
                "provider": "codex",
                "exit_code": 0,
                "duration_ms": 10,
                "stdout_blob_ref": stdout_one,
                "stderr_blob_ref": stderr_one,
                "harness_version": null,
                "timed_out": false
            }),
            json!({
                "schemaVersion": 1,
                "event_type": "step.started",
                "event_id": "evt-step-two",
                "ts": "2026-04-26T07:00:04Z",
                "run_id": run_id,
                "agent_identity": "codex",
                "parent_event_id": "evt-run-started",
                "body_kind": "step_started",
                "step_id": "review"
            }),
            json!({
                "schemaVersion": 1,
                "event_type": "activity.started",
                "event_id": "evt-activity-two",
                "ts": "2026-04-26T07:00:05Z",
                "run_id": run_id,
                "agent_identity": "codex",
                "parent_event_id": "evt-step-two",
                "body_kind": "activity_started",
                "activity_name": "reviewer",
                "activity_type": "agent_loop"
            }),
            json!({
                "schemaVersion": 1,
                "event_type": "cli.invocation.finished",
                "event_id": "evt-cli-two",
                "ts": "2026-04-26T07:00:06Z",
                "run_id": run_id,
                "agent_identity": "codex",
                "parent_event_id": "evt-activity-two",
                "body_kind": "cli_invocation_finished",
                "provider": "claude",
                "exit_code": 0,
                "duration_ms": 20,
                "stdout_blob_ref": stdout_two,
                "stderr_blob_ref": null,
                "harness_version": null,
                "timed_out": false
            }),
        ];
        let jsonl = events
            .iter()
            .map(|event| serde_json::to_string(event).expect("serialize event"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(
            jsonl_dir.join(format!("{run_id}.jsonl")),
            format!("{jsonl}\n"),
        )
        .expect("write jsonl");

        let records = runtime
            .collect_run_cli_invocations(run_id)
            .expect("collect records");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].run_id, run_id);
        assert_eq!(records[0].event_id, "evt-cli-one");
        assert_eq!(records[0].step_id.as_deref(), Some("implement_one"));
        assert_eq!(records[0].step_index, Some(0));
        assert_eq!(records[0].provider.as_deref(), Some("codex"));
        assert_eq!(records[0].stdout, "one stdout\n");
        assert_eq!(records[0].stderr, "one stderr\n");
        assert_eq!(records[0].exit_code, Some(0));
        assert!(!records[0].timed_out);
        assert_eq!(records[0].duration_ms, Some(10));
        assert_eq!(records[1].step_id.as_deref(), Some("review"));
        assert_eq!(records[1].step_index, Some(1));
        assert_eq!(records[1].provider.as_deref(), Some("claude"));
        assert_eq!(records[1].stdout, "two stdout\n");
        assert_eq!(records[1].stderr, "");
    }

    #[test]
    fn missing_run_audit_file_returns_no_cli_invocations() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let records = runtime
            .collect_run_cli_invocations("jrun-missing")
            .expect("collect records");
        assert!(records.is_empty());
    }

    #[test]
    fn malformed_jsonl_and_missing_blobs_are_tolerated() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let audit_root = runtime.data_root().join("state").join("audit");
        let jsonl_dir = audit_root.join("v2_loop");
        std::fs::create_dir_all(&jsonl_dir).expect("create jsonl dir");
        let run_id = "jrun-tolerant";
        std::fs::write(
            jsonl_dir.join(format!("{run_id}.jsonl")),
            format!(
                "{}\nnot-json\n{}\n",
                json!({
                    "event_id": "evt-step",
                    "ts": "2026-04-26T07:00:01Z",
                    "run_id": run_id,
                    "body_kind": "step_started",
                    "step_id": "implement"
                }),
                json!({
                    "event_id": "evt-cli",
                    "ts": "2026-04-26T07:00:02Z",
                    "run_id": run_id,
                    "parent_event_id": "evt-step",
                    "body_kind": "cli_invocation_finished",
                    "provider": "codex",
                    "exit_code": 1,
                    "duration_ms": 42,
                    "stdout_blob_ref": "aa/missing",
                    "stderr_blob_ref": "error:writer-failed",
                    "timed_out": true
                })
            ),
        )
        .expect("write jsonl");

        let events = runtime
            .collect_run_audit_events(run_id)
            .expect("collect events");
        assert_eq!(events.len(), 2);

        let records = runtime
            .collect_run_cli_invocations(run_id)
            .expect("collect records");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].step_index, Some(0));
        assert_eq!(records[0].stdout, "");
        assert_eq!(records[0].stderr, "");
        assert_eq!(records[0].exit_code, Some(1));
        assert!(records[0].timed_out);
    }
}
