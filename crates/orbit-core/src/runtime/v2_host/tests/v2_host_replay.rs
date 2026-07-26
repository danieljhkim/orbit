//! Replay-backed sibling tests for `mod.rs`.
//!
//! These drive the HTTP agent loop through `orbit-engine`'s scripted replay
//! transport, which [ORB-10414] made an explicit, default-off cargo feature:
//! `replay_active()` returns `false` unless `orbit-engine/replay` is selected,
//! so without the feature `drive_agent_loop` falls through to the live
//! Anthropic transport and demands a real credential. They therefore live
//! behind orbit-core's own `replay` feature (which forwards to
//! `orbit-engine/replay`) and run in the dedicated opt-in guardrail pass, the
//! same way orbit-engine's `tests/agent_loop_driver.rs` does. [ORB-10434]

use std::sync::{Mutex, MutexGuard, OnceLock};

use orbit_common::types::activity_job::{AgentLoopSpec, Backend, OnDenial, Provider};
use orbit_common::types::{TaskPriority, TaskStatus, TaskType};
use orbit_engine::{V2AuditWriter, drive_agent_loop, reset_replay_transport};
use tempfile::NamedTempFile;

use super::super::test_support::{runtime_with_workspace_layout, seed_list_backlog_task};
use super::super::*;

fn replay_env_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct ReplayFixtureGuard {
    prior: Option<String>,
}

impl ReplayFixtureGuard {
    fn set(path: &std::path::Path) -> Self {
        let prior = std::env::var("ORBIT_V2_REPLAY_FIXTURE").ok();
        // SAFETY: replay fixture env mutation is serialized by `replay_env_guard`.
        unsafe {
            std::env::set_var("ORBIT_V2_REPLAY_FIXTURE", path);
        }
        reset_replay_transport();
        Self { prior }
    }
}

impl Drop for ReplayFixtureGuard {
    fn drop(&mut self) {
        reset_replay_transport();
        // SAFETY: replay fixture env mutation is serialized by `replay_env_guard`.
        unsafe {
            match &self.prior {
                Some(value) => std::env::set_var("ORBIT_V2_REPLAY_FIXTURE", value),
                None => std::env::remove_var("ORBIT_V2_REPLAY_FIXTURE"),
            }
        }
    }
}

fn write_replay_fixture(value: Value) -> NamedTempFile {
    let file = NamedTempFile::new().expect("fixture temp file");
    std::fs::write(
        file.path(),
        serde_json::to_vec(&value).expect("serialize replay fixture"),
    )
    .expect("write replay fixture");
    file
}

#[test]
fn http_agent_loop_tool_update_persists_runtime_identity_family() {
    let _lock = replay_env_guard();
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let task = seed_list_backlog_task(
        &runtime,
        "runtime identity regression",
        TaskStatus::InProgress,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        Vec::new(),
    );
    let fixture = write_replay_fixture(serde_json::json!({
        "turns": [
            {
                "content": [{
                    "kind": "tool_use",
                    "id": "toolu_identity_update",
                    "name": "orbit.task.update",
                    "input": {
                        "id": task.id.clone(),
                        "status": "review",
                        "execution_summary": "Identity regression covered.",
                        "model": "grok-build"
                    }
                }],
                "stop_reason": "tool_use"
            },
            {
                "content": [{ "kind": "text", "text": "done" }],
                "stop_reason": "end_turn"
            }
        ]
    }));
    let _guard = ReplayFixtureGuard::set(fixture.path());
    let audit_dir = tempfile::tempdir().expect("audit tempdir");
    let audit = V2AuditWriter::with_disk_sinks(
        audit_dir.path(),
        Store::open_in_memory().expect("audit store"),
        "ws_test",
        "http-identity-regression",
        format!("claude:{}", orbit_common::test_fixtures::TEST_CLAUDE_MODEL),
        None,
    )
    .expect("audit writer");
    let spec = AgentLoopSpec {
        instruction: "exercise tool identity".to_string(),
        tools: vec!["orbit.task.update".to_string()],
        on_denial: OnDenial::Terminate,
        model: Some(orbit_common::test_fixtures::TEST_CLAUDE_MODEL.to_string()),
        max_iterations: 2,
        backend: Backend::Http,
        provider: Provider::Claude,
        wall_clock_timeout_seconds: 30,
        require_response_envelope: false,
        role: None,
        proc_allowed_programs: None,
    };

    drive_agent_loop(
        &spec,
        None,
        "http-identity-regression",
        audit,
        &serde_json::json!({ "prompt": "update the task" }),
        &runtime,
        None,
    )
    .expect("replay agent loop succeeds");

    let updated = runtime.get_task(&task.id).expect("updated task");
    assert_eq!(updated.implemented_by.as_deref(), Some("claude"));
}
