//! Schema exposure and round-trip call tests for the newly-surfaced fields on
//! `orbit.task.add` (complexity enum, model, dependencies, relations, parent_id,
//! source_task_id, tags, and strict context wording). Mirrors the style of
//! sibling tests under `update/tests/`.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use orbit_common::types::OrbitError;

use super::super::OrbitTaskAddTool;
use crate::{OrbitBuiltinAction, OrbitTaskScope, OrbitToolHost, Tool, ToolContext};

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
        // Simulate success without touching disk (real YAML write exercised in
        // orbit-core integration tests). Return shape compatible with host.
        Ok(json!({ "id": "ORB-TEST", "title": "roundtrip" }))
    }

    fn task_scope(&self) -> OrbitTaskScope {
        OrbitTaskScope::default()
    }
}

fn mk_ctx(host: RecordingHost) -> ToolContext {
    ToolContext {
        cwd: None,
        allowed_tools: vec![],
        orbit_host: Some(Arc::new(host)),
        ..Default::default()
    }
}

#[test]
fn schema_exposes_create_task_documented_fields() {
    let schema = OrbitTaskAddTool.schema();

    let names: Vec<_> = schema.parameters.iter().map(|p| p.name.as_str()).collect();
    for required in [
        "title",
        "description",
        "workspace",
        "complexity",
        "model",
        "dependencies",
        "relations",
        "parent_id",
        "source_task_id",
        "tags",
        "context_files",
        "context",
    ] {
        assert!(
            names.contains(&required),
            "orbit.task.add schema must expose {required}"
        );
    }

    let complexity = schema
        .parameters
        .iter()
        .find(|p| p.name == "complexity")
        .expect("complexity param");
    assert_eq!(complexity.param_type, "string");
    assert!(!complexity.required);
    assert!(complexity.description.contains("low, medium, or hard"));

    let model = schema
        .parameters
        .iter()
        .find(|p| p.name == "model")
        .expect("model param");
    assert_eq!(model.param_type, "string");
    assert!(!model.required);

    let deps = schema
        .parameters
        .iter()
        .find(|p| p.name == "dependencies")
        .expect("dependencies");
    assert_eq!(deps.param_type, "string_list");

    let rels = schema
        .parameters
        .iter()
        .find(|p| p.name == "relations")
        .expect("relations");
    assert_eq!(rels.param_type, "array");

    let tags = schema
        .parameters
        .iter()
        .find(|p| p.name == "tags")
        .expect("tags");
    assert_eq!(tags.param_type, "string_list");

    let parent = schema
        .parameters
        .iter()
        .find(|p| p.name == "parent_id")
        .expect("parent_id");
    assert_eq!(parent.param_type, "string");

    let source = schema
        .parameters
        .iter()
        .find(|p| p.name == "source_task_id")
        .expect("source_task_id");
    assert_eq!(source.param_type, "string");

    // context (the one whose description we rewrote) must use modify-or-delete wording
    let ctx = schema
        .parameters
        .iter()
        .find(|p| p.name == "context")
        .expect("context legacy param");
    assert!(
        ctx.description.contains("modified or deleted")
            && ctx.description.contains("background-reading"),
        "context desc must use skill's modify-or-delete + disallow background: {}",
        ctx.description
    );
}

#[test]
fn add_call_with_all_fields_roundtrips_to_host() {
    let host = RecordingHost::default();
    let ctx = mk_ctx(host.clone());
    let tool = OrbitTaskAddTool;

    let input = json!({
        "title": "Expose fields test",
        "description": "Round-trip coverage for ORB-00234",
        "workspace": "/tmp/test-ws",
        "complexity": "medium",
        "model": "grok",
        "dependencies": ["ORB-00001"],
        "relations": [{"type": "related_to", "target": "ORB-00002"}],
        "parent_id": "ORB-00003",
        "source_task_id": "ORB-00004",
        "tags": ["mcp", "schema"],
        "context_files": ["file:crates/orbit-tools/src/builtin/orbit/task/add.rs"],
        "context": "file:crates/orbit-tools/src/builtin/orbit/task/add/tests/fields.rs",
        "acceptance_criteria": ["MCP schema has enums", "roundtrip reaches host"],
        "plan": "",
        "priority": "medium",
        "type": "chore",
        "status": "proposed"
    });

    let res = tool.execute(&ctx, input.clone()).expect("execute succeeds");
    assert_eq!(res["id"], "ORB-TEST");

    let recorded = host
        .call
        .lock()
        .expect("lock")
        .take()
        .expect("host was called");
    assert_eq!(recorded.action, OrbitBuiltinAction::TaskAdd);
    assert_eq!(recorded.model.as_deref(), Some("grok"));

    // All newly-exposed (and context alias) fields must be present in the payload
    // that reaches the host (which ultimately writes the task YAML).
    let rec_input = recorded.input;
    for key in [
        "complexity",
        "model",
        "dependencies",
        "relations",
        "parent_id",
        "source_task_id",
        "tags",
        "context_files",
        "context",
    ] {
        assert!(
            rec_input.get(key).is_some(),
            "field {key} must survive the call to orbit.task.add and reach host for YAML persistence"
        );
    }
    assert_eq!(rec_input["complexity"], "medium");
    assert_eq!(rec_input["parent_id"], "ORB-00003");
}
