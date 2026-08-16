//! Sibling tests for `identity.rs` (per docs/design-patterns/test_layout.md).
//!
//! Agent/model identity resolution used to reach through the v1
//! `ActivityExecutorRegistry`. [ORB-10395] deleted that registry, so the lookup
//! now reads the executor def store directly. These tests pin that wiring: a
//! `model_pair_override` seeded into the store must be observable through the
//! public `RuntimeHost::resolved_agent_model_pair` surface.

use std::collections::HashMap;

use chrono::Utc;
use orbit_engine::RuntimeHost;
use orbit_types::identity::AgentModelPair;
use orbit_types::workflow::{ExecutorDef, ExecutorType, ModelPairOverride};

use crate::OrbitRuntime;

fn executor_def(name: &str, model_pair_override: Option<ModelPairOverride>) -> ExecutorDef {
    let now = Utc::now();
    ExecutorDef {
        name: name.to_string(),
        executor_type: ExecutorType::DirectAgent,
        command: Some(name.to_string()),
        args: Vec::new(),
        stdout_format: None,
        model_pair_override,
        model_flag: None,
        timeout_seconds: None,
        env: HashMap::new(),
        sandbox: None,
        allow_fallback: false,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn resolved_agent_model_pair_reads_the_executor_def_store() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    runtime
        .upsert_executor_def(&executor_def(
            "claude",
            Some(ModelPairOverride {
                strong: "claude-orchestrator".to_string(),
                weak: "claude-helper".to_string(),
            }),
        ))
        .expect("seed executor def");

    assert_eq!(
        RuntimeHost::resolved_agent_model_pair(&runtime, "claude"),
        Some(AgentModelPair::new("claude-orchestrator", "claude-helper"))
    );
}

#[test]
fn resolved_agent_model_pair_is_none_without_an_override() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    runtime
        .upsert_executor_def(&executor_def("codex", None))
        .expect("seed executor def");

    assert_eq!(
        RuntimeHost::resolved_agent_model_pair(&runtime, "codex"),
        None
    );
}

#[test]
fn resolved_agent_model_pair_is_none_for_an_unregistered_executor() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    assert_eq!(
        RuntimeHost::resolved_agent_model_pair(&runtime, "not-registered"),
        None
    );
}
