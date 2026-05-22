#![allow(missing_docs)]

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::context::AgentRoleConfig;
use orbit_agent::loop_engine::audit::{AuditSink, LoopAuditEvent};
use orbit_common::types::ExecutorSandboxKind;
use orbit_common::types::activity_job::{AgentLoopSpec, AgentRole, Backend, OnDenial, Provider};
use orbit_common::utility::logging::RedactingFields;
#[cfg(target_os = "macos")]
use orbit_exec::sandbox_exec_path;
use orbit_tools::{FsAuditLogger, ToolContext};
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::{Event, Metadata, Subscriber, span};
use tracing_subscriber::{Registry, fmt as tracing_fmt, fmt::MakeWriter, layer::SubscriberExt};

use super::super::super::dispatcher::{
    DispatchError, ResolvedCliExecutor, ResolvedSandbox, V2RuntimeHost,
};
use super::super::supervisor::SpawnOutput;

pub(in crate::activity_job::cli_runner) fn sandbox_for_test() -> ResolvedSandbox {
    ResolvedSandbox {
        kind: ExecutorSandboxKind::MacosSandboxExec,
        fs_profile: orbit_common::types::ResolvedFsProfile {
            name: "default".to_string(),
            read: vec!["/tmp".to_string()],
            modify: vec!["/tmp".to_string()],
        },
        allow_fallback: false,
    }
}

pub(in crate::activity_job::cli_runner) fn sh_args(script: &str) -> Vec<String> {
    vec!["-c".to_string(), script.to_string()]
}

pub(in crate::activity_job::cli_runner) fn capture_events<F>(
    f: F,
) -> (Result<SpawnOutput, String>, Vec<CapturedEvent>)
where
    F: FnOnce() -> Result<SpawnOutput, String>,
{
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = CaptureSubscriber {
        events: Arc::clone(&events),
        next_span_id: AtomicU64::new(1),
    };
    let dispatch = tracing::Dispatch::new(subscriber);
    let result = tracing::dispatcher::with_default(&dispatch, f);
    let events = events.lock().expect("events lock").clone();
    (result, events)
}

pub(in crate::activity_job::cli_runner) fn capture_redacted_tracing_output<F>(
    f: F,
) -> (Result<SpawnOutput, String>, String)
where
    F: FnOnce() -> Result<SpawnOutput, String>,
{
    let writer = BufferMakeWriter::default();
    let buffer = writer.buffer();
    let subscriber = Registry::default().with(
        tracing_fmt::layer()
            .with_ansi(false)
            .with_writer(writer)
            .fmt_fields(RedactingFields::default()),
    );
    let dispatch = tracing::Dispatch::new(subscriber);
    let result = tracing::dispatcher::with_default(&dispatch, f);
    let output = String::from_utf8(buffer.lock().expect("buffer lock").clone())
        .expect("formatted output utf8");
    (result, output)
}

pub(in crate::activity_job::cli_runner) fn assert_event(
    events: &[CapturedEvent],
    stream: &str,
    line: &str,
) {
    assert!(
        events
            .iter()
            .any(|event| event.field("stream") == Some(stream)
                && event.field("line") == Some(line)),
        "missing event stream={stream:?} line={line:?}; captured={events:?}"
    );
}

#[derive(Debug, Clone)]
pub(in crate::activity_job::cli_runner) struct CapturedEvent {
    pub(in crate::activity_job::cli_runner) fields: BTreeMap<String, String>,
}

impl CapturedEvent {
    pub(in crate::activity_job::cli_runner) fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }
}

struct CaptureSubscriber {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
    next_span_id: AtomicU64,
}

impl Subscriber for CaptureSubscriber {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(self.next_span_id.fetch_add(1, Ordering::Relaxed))
    }

    fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}

    fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut visitor = FieldCapture::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("events lock")
            .push(CapturedEvent {
                fields: visitor.fields,
            });
    }

    fn enter(&self, _span: &span::Id) {}

    fn exit(&self, _span: &span::Id) {}
}

#[derive(Default)]
struct FieldCapture {
    pub(in crate::activity_job::cli_runner) fields: BTreeMap<String, String>,
}

impl Visit for FieldCapture {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_string(), value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.fields
            .insert(field.name().to_string(), format!("{value:?}"));
    }
}

#[derive(Clone, Default)]
struct BufferMakeWriter {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl BufferMakeWriter {
    fn buffer(&self) -> Arc<Mutex<Vec<u8>>> {
        Arc::clone(&self.buffer)
    }
}

impl<'writer> MakeWriter<'writer> for BufferMakeWriter {
    type Writer = BufferWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        BufferWriter {
            buffer: Arc::clone(&self.buffer),
        }
    }
}

struct BufferWriter {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl Write for BufferWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer
            .lock()
            .expect("buffer lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
pub(in crate::activity_job::cli_runner) struct RecordingSink {
    events: Mutex<Vec<LoopAuditEvent>>,
    blobs: Mutex<Vec<(String, Vec<u8>)>>,
}

impl RecordingSink {
    pub(in crate::activity_job::cli_runner) fn blob(&self, reference: &str) -> Option<Vec<u8>> {
        self.blobs
            .lock()
            .expect("blobs lock")
            .iter()
            .find_map(|(id, bytes)| {
                if id == reference {
                    Some(bytes.clone())
                } else {
                    None
                }
            })
    }
}

impl AuditSink for RecordingSink {
    fn emit(&self, event: &LoopAuditEvent) {
        self.events.lock().expect("events lock").push(event.clone());
    }

    fn write_blob(&self, content: &[u8]) -> String {
        let mut blobs = self.blobs.lock().expect("blobs lock");
        let reference = format!("blob-{}", blobs.len() + 1);
        blobs.push((reference.clone(), content.to_vec()));
        reference
    }
}

pub(in crate::activity_job::cli_runner) struct TestHost {
    pub(in crate::activity_job::cli_runner) command: String,
    pub(in crate::activity_job::cli_runner) executor_args: Vec<String>,
    pub(in crate::activity_job::cli_runner) provider_config: HashMap<String, String>,
    pub(in crate::activity_job::cli_runner) sandbox: Option<ResolvedSandbox>,
    pub(in crate::activity_job::cli_runner) task_context: Option<Value>,
}

impl TestHost {
    pub(in crate::activity_job::cli_runner) fn with_command(command: String) -> Self {
        Self {
            command,
            executor_args: Vec::new(),
            provider_config: HashMap::new(),
            sandbox: None,
            task_context: None,
        }
    }
}

impl V2RuntimeHost for TestHost {
    fn run_deterministic(
        &self,
        _action: &str,
        _config: &Value,
        _input: &Value,
        _tool_context: ToolContext,
    ) -> Result<Value, DispatchError> {
        unreachable!("not used by cli runner tests")
    }

    fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
        Ok(String::new())
    }

    fn resolve_cli_executor(&self, _provider: &str) -> Result<ResolvedCliExecutor, DispatchError> {
        Ok(ResolvedCliExecutor {
            command: self.command.clone(),
            args: self.executor_args.clone(),
        })
    }

    fn provider_cli_config(&self, _provider: &str) -> HashMap<String, String> {
        self.provider_config.clone()
    }

    fn resolve_executor_sandbox(
        &self,
        _provider: &str,
        _fs_profile: Option<&str>,
        _subprocess_cwd: Option<&Path>,
    ) -> Result<Option<ResolvedSandbox>, DispatchError> {
        Ok(self.sandbox.clone())
    }

    fn task_context_for_agent_input(&self, _input: &Value) -> Result<Option<Value>, DispatchError> {
        Ok(self.task_context.clone())
    }

    fn agent_role_config_for_input(
        &self,
        role: AgentRole,
        input: &Value,
    ) -> Option<AgentRoleConfig> {
        let crew = input.get("crew").and_then(|v| v.as_str()).unwrap_or("");
        if crew != "mixed-fixture" {
            return None;
        }
        match role {
            AgentRole::Planner => Some(AgentRoleConfig {
                provider: Some(Provider::Claude),
                model: Some("claude-opus-4-7".to_string()),
                backend: None,
            }),
            AgentRole::Implementer => Some(AgentRoleConfig {
                provider: Some(Provider::Codex),
                model: Some("gpt-5.5".to_string()),
                backend: None,
            }),
            _ => None,
        }
    }

    fn tool_context_for_activity(
        &self,
        _run_id: Option<&str>,
        _fs_profile: Option<&str>,
        _fs_audit: Option<Arc<dyn FsAuditLogger>>,
    ) -> ToolContext {
        ToolContext::default()
    }
}

pub(in crate::activity_job::cli_runner) fn test_agent_loop_spec(
    timeout: Duration,
) -> AgentLoopSpec {
    AgentLoopSpec {
        instruction: String::new(),
        tools: Vec::new(),
        on_denial: OnDenial::Terminate,
        model: None,
        max_iterations: 1,
        backend: Backend::Cli,
        provider: Provider::Codex,
        wall_clock_timeout_seconds: timeout.as_secs(),
        role: None,
    }
}

pub(in crate::activity_job::cli_runner) fn test_agent_loop_spec_for(
    provider: &str,
    timeout: Duration,
) -> AgentLoopSpec {
    let provider = match provider {
        "claude" => Provider::Claude,
        "codex" => Provider::Codex,
        "gemini" => Provider::Gemini,
        "grok" => Provider::Grok,
        other => panic!("unsupported provider for test: {other}"),
    };
    AgentLoopSpec {
        instruction: String::new(),
        tools: Vec::new(),
        on_denial: OnDenial::Terminate,
        model: None,
        max_iterations: 1,
        backend: Backend::Cli,
        provider,
        wall_clock_timeout_seconds: timeout.as_secs(),
        role: None,
    }
}

pub(in crate::activity_job::cli_runner) fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write script");
    make_executable(path);
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path).expect("script metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("script permissions");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

#[cfg(target_os = "macos")]
pub(in crate::activity_job::cli_runner) fn sandbox_exec_can_apply_for_test() -> bool {
    let Some(path) = sandbox_exec_path() else {
        return false;
    };
    Command::new(path)
        .args(["-p", "(version 1)\n(allow default)\n", "/usr/bin/true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
