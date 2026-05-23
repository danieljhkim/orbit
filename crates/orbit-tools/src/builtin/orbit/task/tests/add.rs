//! Schema exposure and compatibility tests for `orbit.task.add`.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use orbit_common::types::{OrbitError, RETIRED_TASK_ADD_INPUT_FIELDS, ToolSessionContext};

use super::super::add::OrbitTaskAddTool;
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

fn capture_warnings<F, T>(f: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    use std::io::{self, Write};
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone)]
    struct CaptureMakeWriter(Arc<Mutex<Vec<u8>>>);
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl<'a> MakeWriter<'a> for CaptureMakeWriter {
        type Writer = CaptureWriter;

        fn make_writer(&'a self) -> Self::Writer {
            CaptureWriter(Arc::clone(&self.0))
        }
    }

    impl Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("capture lock").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(CaptureMakeWriter(Arc::clone(&buffer)))
        .with_max_level(LevelFilter::WARN)
        .with_target(true)
        .with_ansi(false)
        .without_time()
        .finish();
    let result = tracing::subscriber::with_default(subscriber, f);
    let logs =
        String::from_utf8(buffer.lock().expect("capture buffer lock").clone()).expect("utf8 logs");
    (result, logs)
}

fn capture_info<F, T>(f: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    use std::io::{self, Write};
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone)]
    struct CaptureMakeWriter(Arc<Mutex<Vec<u8>>>);
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl<'a> MakeWriter<'a> for CaptureMakeWriter {
        type Writer = CaptureWriter;

        fn make_writer(&'a self) -> Self::Writer {
            CaptureWriter(Arc::clone(&self.0))
        }
    }

    impl Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("capture lock").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(CaptureMakeWriter(Arc::clone(&buffer)))
        .with_max_level(LevelFilter::INFO)
        .with_target(true)
        .with_ansi(false)
        .without_time()
        .finish();
    let result = tracing::subscriber::with_default(subscriber, f);
    let logs =
        String::from_utf8(buffer.lock().expect("capture buffer lock").clone()).expect("utf8 logs");
    (result, logs)
}

#[test]
fn schema_exposes_only_trimmed_create_task_fields() {
    let schema = OrbitTaskAddTool.schema();

    let names: Vec<_> = schema.parameters.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "title",
            "description",
            "workspace",
            "acceptance_criteria",
            "tags",
            "context_files",
            "priority",
            "complexity",
            "type",
            "relations",
            "model",
        ]
    );

    let required: Vec<_> = schema
        .parameters
        .iter()
        .filter(|param| param.required)
        .map(|param| param.name.as_str())
        .collect();
    assert_eq!(required, vec!["title", "description"]);
    let workspace = schema
        .parameters
        .iter()
        .find(|param| param.name == "workspace")
        .expect("workspace param");
    assert!(!workspace.required);

    for removed in RETIRED_TASK_ADD_INPUT_FIELDS {
        assert!(
            !names.contains(removed),
            "orbit.task.add schema must not expose retired field {removed}"
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

    let relations = schema
        .parameters
        .iter()
        .find(|p| p.name == "relations")
        .expect("relations");
    assert_eq!(relations.param_type, "array");

    let context_files = schema
        .parameters
        .iter()
        .find(|p| p.name == "context_files")
        .expect("context_files");
    assert_eq!(context_files.param_type, "string_list");
}

#[test]
fn add_call_with_retired_fields_warns_once_and_ignores_them() {
    let host = RecordingHost::default();
    let ctx = mk_ctx(host.clone());
    let tool = OrbitTaskAddTool;

    let input = json!({
        "title": "Trimmed add fields test",
        "description": "Compatibility coverage for ORB-00255",
        "workspace": "/tmp/test-ws",
        "acceptance_criteria": ["MCP schema is trimmed", "retired fields are ignored"],
        "tags": ["mcp", "schema"],
        "context_files": ["file:crates/orbit-tools/src/builtin/orbit/task/add.rs"],
        "priority": "medium",
        "complexity": "medium",
        "type": "chore",
        "relations": [{"type": "related_to", "target": "ORB-00002"}],
        "model": "grok",
        "plan": "ignored plan",
        "status": "done",
        "crew": "ignored-crew",
        "parent_id": "ORB-00003",
        "source_task_id": "ORB-00004",
        "external_refs": [{"system": "ENG", "id": "123"}],
        "context": "file:legacy-alias.rs",
        "comment": "ignored comment",
        "dependencies": ["ORB-00001"]
    });

    let (res, logs) = capture_warnings(|| tool.execute(&ctx, input).expect("execute succeeds"));
    assert_eq!(res["id"], "ORB-TEST");
    assert_eq!(
        logs.matches("ignored retired orbit.task.add fields")
            .count(),
        1,
        "compatibility warning must fire once per execute call: {logs}"
    );
    for removed in RETIRED_TASK_ADD_INPUT_FIELDS {
        assert!(
            logs.contains(*removed),
            "compatibility warning must name retired field {removed}: {logs}"
        );
    }

    let recorded = host
        .call
        .lock()
        .expect("lock")
        .take()
        .expect("host was called");
    assert_eq!(recorded.action, OrbitBuiltinAction::TaskAdd);
    assert_eq!(recorded.agent.as_deref(), None);
    assert_eq!(recorded.model.as_deref(), Some("grok"));

    let rec_input = recorded.input;
    for removed in RETIRED_TASK_ADD_INPUT_FIELDS {
        assert!(
            rec_input.get(*removed).is_none(),
            "retired field {removed} must be stripped before host execution"
        );
    }
    for kept in [
        "title",
        "description",
        "workspace",
        "acceptance_criteria",
        "tags",
        "context_files",
        "priority",
        "complexity",
        "type",
        "relations",
        "model",
    ] {
        assert!(
            rec_input.get(kept).is_some(),
            "kept field {kept} must survive host execution"
        );
    }
    assert_eq!(rec_input["complexity"], "medium");
    assert_eq!(
        rec_input["context_files"][0],
        "file:crates/orbit-tools/src/builtin/orbit/task/add.rs"
    );
}

#[test]
fn add_call_uses_session_workspace_when_input_omits_workspace() {
    let host = RecordingHost::default();
    let mut ctx = mk_ctx(host.clone());
    ctx.session_context = ToolSessionContext::with_workspace("/tmp/canonical-ws");
    let tool = OrbitTaskAddTool;

    tool.execute(
        &ctx,
        json!({
            "title": "Ambient workspace test",
            "description": "MCP session context supplies workspace",
            "model": "codex"
        }),
    )
    .expect("session workspace should satisfy workspace");

    let recorded = host
        .call
        .lock()
        .expect("lock")
        .take()
        .expect("host was called");
    assert_eq!(recorded.input["workspace"], "/tmp/canonical-ws");
}

#[test]
fn explicit_workspace_overrides_mismatched_session_workspace() {
    let host = RecordingHost::default();
    let mut ctx = mk_ctx(host.clone());
    ctx.session_context = ToolSessionContext::with_workspace("/tmp/session-ws");
    let tool = OrbitTaskAddTool;

    let (_res, logs) = capture_info(|| {
        tool.execute(
            &ctx,
            json!({
                "title": "Explicit workspace wins",
                "description": "The caller can override session context",
                "workspace": "/tmp/explicit-ws",
                "model": "codex"
            }),
        )
        .expect("explicit workspace should win")
    });

    let recorded = host
        .call
        .lock()
        .expect("lock")
        .take()
        .expect("host was called");
    assert_eq!(recorded.input["workspace"], "/tmp/explicit-ws");
    assert!(
        logs.contains("explicit workspace overrides MCP session context"),
        "mismatch should be logged at info level: {logs}"
    );
}

#[test]
fn add_call_missing_required_fields_returns_required_field_error() {
    let cases = [
        (
            "title",
            json!({
                "description": "missing title",
                "workspace": "/tmp/test-ws"
            }),
        ),
        (
            "description",
            json!({
                "title": "missing description",
                "workspace": "/tmp/test-ws"
            }),
        ),
    ];

    for (missing, input) in cases {
        let host = RecordingHost::default();
        let ctx = mk_ctx(host.clone());
        let err = OrbitTaskAddTool
            .execute(&ctx, input)
            .expect_err("missing required field should fail");
        match err {
            OrbitError::InvalidInput(message) => {
                assert_eq!(message, format!("missing `{missing}`"));
            }
            other => panic!("unexpected error for missing {missing}: {other}"),
        }
        assert!(
            host.call.lock().expect("lock").is_none(),
            "host must not be called when {missing} is missing"
        );
    }
}

#[test]
fn add_call_missing_workspace_without_session_context_returns_clear_error() {
    let host = RecordingHost::default();
    let ctx = mk_ctx(host.clone());
    let err = OrbitTaskAddTool
        .execute(
            &ctx,
            json!({
                "title": "missing workspace",
                "description": "missing workspace"
            }),
        )
        .expect_err("missing workspace and session context should fail");
    match err {
        OrbitError::InvalidInput(message) => {
            assert!(message.contains("missing `workspace`"), "{message}");
            assert!(message.contains("MCP session"), "{message}");
        }
        other => panic!("unexpected error for missing workspace: {other}"),
    }
    assert!(
        host.call.lock().expect("lock").is_none(),
        "host must not be called when workspace cannot be resolved"
    );
}
