//! Execution and source path resolution tests for task.artifact_put.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use orbit_common::types::OrbitError;

use super::super::*;
use crate::{OrbitTaskScope, OrbitToolHost};

#[derive(Clone, Default)]
struct RecordingHost {
    call: Arc<Mutex<Option<RecordedCall>>>,
}

#[derive(Debug)]
struct RecordedCall {
    action: OrbitBuiltinAction,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
}

impl OrbitToolHost for RecordingHost {
    fn execute(
        &self,
        action: OrbitBuiltinAction,
        input: Value,
        agent: Option<String>,
        model: Option<String>,
        _reservation_owner: Option<crate::ReservationOwnerContext>,
    ) -> Result<Value, OrbitError> {
        *self.call.lock().expect("record call") = Some(RecordedCall {
            action,
            input,
            agent,
            model,
        });
        Ok(json!({ "ok": true }))
    }

    fn task_scope(&self) -> OrbitTaskScope {
        OrbitTaskScope::default()
    }
}

#[test]
fn artifact_put_reads_relative_source_and_delegates_to_task_update() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("summary.md");
    std::fs::write(&source, "done\n").expect("write source");
    let host = RecordingHost::default();
    let ctx = ToolContext {
        cwd: Some(dir.path().to_string_lossy().into_owned()),
        orbit_host: Some(Arc::new(host.clone())),
        ..Default::default()
    };

    let output = OrbitTaskArtifactPutTool
        .execute(
            &ctx,
            json!({
                "id": "ORB-00001",
                "source_path": "summary.md",
                "path": "reports/summary.md",
                "agent": "codex",
                "model": "gpt-5"
            }),
        )
        .expect("execute tool");

    assert_eq!(output, json!({ "ok": true }));
    let call = host.call.lock().expect("recorded call").take().unwrap();
    assert_eq!(call.action, OrbitBuiltinAction::TaskUpdate);
    assert_eq!(call.agent.as_deref(), Some("codex"));
    assert_eq!(call.model.as_deref(), Some("gpt-5"));
    assert_eq!(call.input["id"], "ORB-00001");
    assert_eq!(call.input["artifacts"][0]["path"], "reports/summary.md");
    assert_eq!(
        call.input["artifacts"][0]["content"],
        json!([100, 111, 110, 101, 10])
    );
    assert!(call.input.get("source_path").is_none());
}
