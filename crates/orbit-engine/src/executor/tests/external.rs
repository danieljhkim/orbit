#![allow(missing_docs)]

//! End-to-end conformance for the `external` executor (External Executor
//! Protocol v1). These drive the shared subprocess transport directly with a
//! lightweight stub host and a `/bin/sh` fixture that speaks v1, so they run
//! the real `ExecRequest` -> spawn -> exit-code-mapping path without the full
//! runtime host. See ADR-0196 / [ORB-00384].

use std::collections::HashMap;

use chrono::Utc;
use orbit_common::types::{
    Activity, ExecutorDef, ExecutorSandboxKind, ExecutorType, JobRunState, OrbitError,
};
use serde_json::json;

use super::super::ActivityExecutor;
use super::super::direct_agent::{build_subprocess_exec_request, run_subprocess_executor};
use super::super::registry::ActivityExecutorRegistry;
use crate::context::{
    AGENT_INVOCATION_FAILED, AgentProtocolHost, EnvironmentHost, ExecutionContext,
};

/// Minimal host implementing only the surface the subprocess transport touches:
/// it hands back a fixed Protocol v1 request envelope and a deterministic
/// PATH-only environment (so the fixture can resolve `cat` while keeping the
/// `ExecRequest` comparison stable).
struct StubHost {
    envelope: Vec<u8>,
}

impl EnvironmentHost for StubHost {
    fn agent_provider_config(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    fn execution_env_inherit(&self) -> bool {
        false
    }

    fn hydrated_env_allowlist(&self, _env_extra: &[String]) -> Vec<(String, String)> {
        match std::env::var("PATH") {
            Ok(path) => vec![("PATH".to_string(), path)],
            Err(_) => Vec::new(),
        }
    }

    fn orbit_root(&self) -> Option<String> {
        None
    }

    fn cli_command_environment(&self, _env_extra: &[String]) -> Vec<(String, String)> {
        Vec::new()
    }

    fn missing_required_environment_vars(&self, _required_env_vars: &[&str]) -> Vec<String> {
        Vec::new()
    }
}

impl AgentProtocolHost for StubHost {
    fn build_agent_stdin_envelope_payload(
        &self,
        _execution: &ExecutionContext,
    ) -> Result<Vec<u8>, OrbitError> {
        Ok(self.envelope.clone())
    }
}

fn stub_host() -> StubHost {
    StubHost {
        envelope: br#"{"schemaVersion":1,"kind":"agent_request"}"#.to_vec(),
    }
}

fn activity() -> Activity {
    let now = Utc::now();
    Activity {
        id: "ext-conformance".to_string(),
        spec_type: "agent_invoke".to_string(),
        description: String::new(),
        input_schema_json: json!({}),
        output_schema_json: json!({}),
        spec_config: json!({}),
        tools: Vec::new(),
        proc_allowed_programs: Vec::new(),
        executor: None,
        workspace_path: None,
        created_by: None,
        is_active: true,
        created_at: now,
        updated_at: now,
    }
}

fn execution() -> ExecutionContext {
    ExecutionContext {
        activity: activity(),
        job: None,
        agent_cli: "external".to_string(),
        model: None,
        timeout_seconds: 30,
        env_extra: Vec::new(),
        env_set: HashMap::new(),
        input: json!({}),
        debug: false,
        steps_outputs: HashMap::new(),
        run_id: None,
        step_index: None,
        state_dir: None,
    }
}

fn external_def(command: &str, args: &[&str]) -> ExecutorDef {
    let now = Utc::now();
    ExecutorDef {
        name: "acme-harness".to_string(),
        executor_type: ExecutorType::External,
        command: Some(command.to_string()),
        args: args.iter().map(|value| (*value).to_string()).collect(),
        stdout_format: None,
        model_pair_override: None,
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
fn load_from_defs_registers_external_executor_by_name() {
    let mut registry = ActivityExecutorRegistry::new();
    registry.load_from_defs(&[external_def("/bin/sh", &["-c", "exit 0"])]);

    let executor = registry
        .get("acme-harness")
        .expect("external def should register under its name");
    assert_eq!(executor.spec_type(), "external");
}

#[test]
fn load_from_defs_skips_external_def_without_command() {
    let mut def = external_def("ignored", &[]);
    def.command = None;

    let mut registry = ActivityExecutorRegistry::new();
    registry.load_from_defs(&[def]);

    assert!(
        registry.get("acme-harness").is_none(),
        "command-less external def must be skipped, not registered"
    );
}

#[test]
fn external_protocol_v1_success_reports_success_outcome() {
    let host = stub_host();
    // Fixture speaks v1: drains the request envelope from stdin, exits 0.
    let def = external_def("/bin/sh", &["-c", "cat >/dev/null; exit 0"]);

    let outcome = run_subprocess_executor(&def, &host, &execution());

    assert_eq!(outcome.state, JobRunState::Success);
    assert_eq!(outcome.error_code, None);
}

#[test]
fn external_protocol_v1_violation_reports_failed_outcome() {
    let host = stub_host();
    // Fixture violates the protocol's "exit 0 on success" contract: it drains
    // the request, writes a diagnostic to stderr, and exits non-zero.
    let def = external_def(
        "/bin/sh",
        &[
            "-c",
            "cat >/dev/null; echo 'protocol violation' 1>&2; exit 7",
        ],
    );

    let outcome = run_subprocess_executor(&def, &host, &execution());

    assert_eq!(outcome.state, JobRunState::Failed);
    assert_eq!(outcome.error_code.as_deref(), Some(AGENT_INVOCATION_FAILED));
    assert!(
        outcome
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("protocol violation"),
        "stderr should surface as the failure message: {:?}",
        outcome.error_message
    );
}

#[test]
fn external_and_direct_agent_build_identical_exec_request() {
    // Tier 1 parity: `external` reuses the `direct_agent` transport verbatim, so
    // identical command/args/env/name produce a byte-identical ExecRequest. A
    // `sandbox` field on the external def is inert in Tier 1 — the transport
    // ignores it, exactly as direct_agent does (no silent sandbox behavior; real
    // FsProfile->OS sandbox is deferred to Tier 2). See ADR-0196.
    let host = stub_host();
    let execution = execution();

    let mut external = external_def("acme-harness", &["run", "--json"]);
    let direct = {
        let mut def = external.clone();
        def.executor_type = ExecutorType::DirectAgent;
        def
    };
    external.sandbox = Some(ExecutorSandboxKind::MacosSandboxExec);

    let req_external =
        build_subprocess_exec_request(&external, &host, &execution).expect("external request");
    let req_direct =
        build_subprocess_exec_request(&direct, &host, &execution).expect("direct_agent request");

    assert_eq!(req_external.program, req_direct.program);
    assert_eq!(req_external.args, req_direct.args);
    assert_eq!(req_external.current_dir, req_direct.current_dir);
    assert_eq!(req_external.timeout_ms, req_direct.timeout_ms);
    assert_eq!(req_external.stdin_mode, req_direct.stdin_mode);
    assert_eq!(req_external.environment_mode, req_direct.environment_mode);
    assert_eq!(req_external.debug, req_direct.debug);
}
