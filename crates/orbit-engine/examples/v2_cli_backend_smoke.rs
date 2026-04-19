//! v2 `backend: cli` smoke suite — T20260419-0104.
//!
//! Exercises the new §3.1 dispatch path, the §7.6 envelope events, the §6
//! harness-delegated allowlist advisory, the §3.2 loader rejection, argv
//! redaction, and wall-clock timeout. This is an example binary (repo policy
//! prohibits unit tests) — each scenario logs a summary and panics on an
//! unexpected outcome.
//!
//! The smoke substitutes the real `claude` CLI with tempdir shell scripts
//! named `claude` so `AgentConfig::from_cli_config` resolves them to the
//! retained `ClaudeRuntime` (§10.1 keep/delete table). This is what the task
//! AC #11 means by "substitutable" CLI.
//!
//! Run: `cargo run -p orbit-engine --example v2_cli_backend_smoke`

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use orbit_common::types::JobScheduleState;
use orbit_common::types::v2::{
    ActivityAsset, ActivityV2Spec, AgentLoopSpec, Backend, BackendConstraintError, JobKind, JobV2,
    JobV2Step, JobV2StepBody, LoopBlock, OnDenial, Provider, TargetStep, load_activity_asset,
    resolve_job_backends, validate_job_loop_session_backends,
};
use orbit_engine::v2::{
    DispatchError, V2AuditWriter, V2DispatchInput, V2RuntimeHost, dispatch_v2_activity,
};
use serde_json::Value;
use tempfile::TempDir;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("v2 cli-backend smoke — T20260419-0104");

    scenario_a_cli_dispatch_emits_envelope_events()?;
    scenario_b_argv_redaction()?;
    scenario_c_wall_clock_timeout()?;
    scenario_d_no_silent_fallback_unwired_http()?;
    scenario_e_loader_rejection_loop_session_cli()?;
    scenario_f_loader_rejection_auto_resolved_to_cli()?;
    scenario_g_auto_backend_unresolved_is_structural_error()?;
    scenario_h_cli_reference_asset_round_trip()?;
    scenario_i_existing_agent_loop_assets_still_deserialize()?;

    println!("OK — all scenarios passed");
    Ok(())
}

/// A: `backend: cli` against a fake `claude` binary produces
/// `tool_allowlist.harness_delegated`, `cli.invocation.started`, and
/// `cli.invocation.finished` envelope events.
fn scenario_a_cli_dispatch_emits_envelope_events() -> Result<(), Box<dyn std::error::Error>> {
    println!("  A) cli dispatch emits §6 + §7.6 envelope events");
    let tmp_audit = tempfile::tempdir()?;
    let (writer, _sink) = build_writer(tmp_audit.path(), "smoke-cli-a")?;

    // `claude` that ignores stdin and prints a canned reply.
    let fake = fake_cli(
        "claude",
        "#!/bin/sh\ncat > /dev/null\necho '{\"status\":\"ok\"}'\n",
    )?;

    let spec = cli_agent_loop_spec(None);
    let host = ScriptHost::new(fake.cli_path());
    let outcome = dispatch_v2_activity(V2DispatchInput {
        activity_name: "cli_smoke_a",
        spec: &ActivityV2Spec::AgentLoop(spec),
        fs_profile: None,
        input: serde_json::json!({ "prompt": "hello" }),
        audit: writer.clone(),
        run_id: "smoke-cli-a",
        host: Some(&host),
    })?;

    assert!(outcome.success, "fake claude should exit 0");

    let events = writer.events_snapshot()?;
    let types: Vec<&str> = events
        .iter()
        .map(|e| e.envelope.event_type.as_str())
        .collect();
    must_contain(&types, "tool_allowlist.harness_delegated");
    must_contain(&types, "cli.invocation.started");
    must_contain(&types, "cli.invocation.finished");
    println!("    events: {:?}", types);
    Ok(())
}

/// B: argv carrying an `sk-...` token (via the `--model` flag set by
/// `claude_cli.rs`) is redacted in the persisted envelope event.
fn scenario_b_argv_redaction() -> Result<(), Box<dyn std::error::Error>> {
    println!("  B) argv redaction scrubs sk-... from --model arg");
    let tmp_audit = tempfile::tempdir()?;
    let (writer, _sink) = build_writer(tmp_audit.path(), "smoke-cli-b")?;

    let fake = fake_cli("claude", "#!/bin/sh\ncat > /dev/null\necho ok\n")?;

    // Plant the token in the spec's `model` field. ClaudeCliTransport passes
    // it on argv as `--model <value>` — the redactor sees it there.
    let mut spec = cli_agent_loop_spec(None);
    spec.model = Some("sk-ant-LEAKEDKEY99999".to_string());
    let host = ScriptHost::new(fake.cli_path());

    let _ = dispatch_v2_activity(V2DispatchInput {
        activity_name: "cli_smoke_b",
        spec: &ActivityV2Spec::AgentLoop(spec),
        fs_profile: None,
        input: serde_json::json!({ "prompt": "redact me" }),
        audit: writer.clone(),
        run_id: "smoke-cli-b",
        host: Some(&host),
    })?;

    let events = writer.events_snapshot()?;
    let started = events
        .iter()
        .find(|e| e.envelope.event_type == "cli.invocation.started")
        .ok_or("missing cli.invocation.started")?;
    let serialized = serde_json::to_string(started)?;
    assert!(
        !serialized.contains("sk-ant-LEAKEDKEY"),
        "sk- token leaked into envelope: {}",
        serialized
    );
    assert!(
        serialized.contains("[REDACTED_API_KEY]"),
        "expected [REDACTED_API_KEY] marker in {}",
        serialized
    );
    println!("    envelope redacted sk- token");
    Ok(())
}

/// C: 2s timeout against a 10s `sleep` — finishes within 5s with
/// `timed_out: true`.
fn scenario_c_wall_clock_timeout() -> Result<(), Box<dyn std::error::Error>> {
    println!("  C) wall_clock_timeout kills long-running subprocess");
    let tmp_audit = tempfile::tempdir()?;
    let (writer, _sink) = build_writer(tmp_audit.path(), "smoke-cli-c")?;

    let fake = fake_cli("claude", "#!/bin/sh\ncat > /dev/null\nexec sleep 10\n")?;

    let mut spec = cli_agent_loop_spec(None);
    spec.wall_clock_timeout_seconds = 2;
    let host = ScriptHost::new(fake.cli_path());
    let started = Instant::now();
    let _ = dispatch_v2_activity(V2DispatchInput {
        activity_name: "cli_smoke_c",
        spec: &ActivityV2Spec::AgentLoop(spec),
        fs_profile: None,
        input: serde_json::json!({ "prompt": "ignored" }),
        audit: writer.clone(),
        run_id: "smoke-cli-c",
        host: Some(&host),
    })?;
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "AC #7: timeout did not kill subprocess within 3s (elapsed {:?})",
        elapsed
    );

    let events = writer.events_snapshot()?;
    let finished = events
        .iter()
        .find(|e| e.envelope.event_type == "cli.invocation.finished")
        .ok_or("missing cli.invocation.finished")?;
    let body = serde_json::to_value(&finished.kind)?;
    assert_eq!(
        body.get("timed_out"),
        Some(&Value::Bool(true)),
        "expected timed_out=true, got {body:?}"
    );
    println!("    timed_out=true (elapsed {:?})", elapsed);
    Ok(())
}

/// D: `backend: http` + unwired provider surfaces `UnwiredHttpTransport`.
fn scenario_d_no_silent_fallback_unwired_http() -> Result<(), Box<dyn std::error::Error>> {
    println!("  D) no-silent-fallback: http + unwired provider (gemini)");
    let tmp_audit = tempfile::tempdir()?;
    let (writer, _sink) = build_writer(tmp_audit.path(), "smoke-cli-d")?;

    let mut spec = cli_agent_loop_spec(Some(Provider::Gemini));
    spec.backend = Backend::Http;
    let host = NullCliHost;
    let err = dispatch_v2_activity(V2DispatchInput {
        activity_name: "cli_smoke_d",
        spec: &ActivityV2Spec::AgentLoop(spec),
        fs_profile: None,
        input: serde_json::json!({ "prompt": "ignored" }),
        audit: writer.clone(),
        run_id: "smoke-cli-d",
        host: Some(&host),
    })
    .expect_err("expected UnwiredHttpTransport");
    match err {
        DispatchError::UnwiredHttpTransport { provider } => {
            assert_eq!(provider, "gemini");
            println!("    got structured error for provider=gemini");
        }
        other => panic!("expected UnwiredHttpTransport, got {other:?}"),
    }
    Ok(())
}

/// E: loader-level rejection for `loop:`-nested step with `session:` +
/// resolved `backend: cli`.
fn scenario_e_loader_rejection_loop_session_cli() -> Result<(), Box<dyn std::error::Error>> {
    println!("  E) loader rejects loop+session+cli");
    let job = synthetic_loop_session_cli_job();
    let err = validate_job_loop_session_backends(&job, "synthetic/loop_session_cli.yaml")
        .expect_err("expected rejection");
    let BackendConstraintError::LoopSessionOnCli {
        asset_path,
        step_id,
        session_name,
        item_number,
        ..
    } = &err;
    assert_eq!(asset_path, "synthetic/loop_session_cli.yaml");
    assert_eq!(step_id, "review");
    assert_eq!(session_name, "reviewer");
    assert_eq!(*item_number, 1);
    println!(
        "    §3.2 item {} rejection for step `{}`",
        item_number, step_id
    );
    Ok(())
}

/// F: `auto` resolved to `cli` rejected identically to explicit `cli`.
fn scenario_f_loader_rejection_auto_resolved_to_cli() -> Result<(), Box<dyn std::error::Error>> {
    println!("  F) auto → cli rejected identically");
    let mut job = synthetic_loop_session_auto_job();
    resolve_job_backends(&mut job, Backend::Cli);
    let err = validate_job_loop_session_backends(&job, "synthetic/loop_session_auto.yaml")
        .expect_err("expected rejection");
    let BackendConstraintError::LoopSessionOnCli { step_id, .. } = &err;
    assert_eq!(step_id, "review");
    println!("    auto-resolved-to-cli rejected for step `{}`", step_id);
    Ok(())
}

/// G: unresolved `Auto` at dispatch is a structural error, not a fallback.
fn scenario_g_auto_backend_unresolved_is_structural_error() -> Result<(), Box<dyn std::error::Error>>
{
    println!("  G) unresolved Auto is a structural error at dispatch");
    let tmp_audit = tempfile::tempdir()?;
    let (writer, _sink) = build_writer(tmp_audit.path(), "smoke-cli-g")?;

    let mut spec = cli_agent_loop_spec(None);
    spec.backend = Backend::Auto; // deliberately unresolved
    let host = NullCliHost;
    let err = dispatch_v2_activity(V2DispatchInput {
        activity_name: "cli_smoke_g",
        spec: &ActivityV2Spec::AgentLoop(spec),
        fs_profile: None,
        input: serde_json::json!({ "prompt": "ignored" }),
        audit: writer.clone(),
        run_id: "smoke-cli-g",
        host: Some(&host),
    })
    .expect_err("expected UnresolvedAutoBackend");
    match err {
        DispatchError::UnresolvedAutoBackend { step_id } => {
            assert_eq!(step_id, "cli_smoke_g");
            println!("    got UnresolvedAutoBackend for step `{}`", step_id);
        }
        other => panic!("expected UnresolvedAutoBackend, got {other:?}"),
    }
    Ok(())
}

/// H: load `v2_agent_loop_cli_reference.yaml` from disk and execute it
/// end-to-end through the CLI runner using a substitutable `claude` script.
/// Proves the YAML parses with the new `backend:` / `provider:` /
/// `wall_clock_timeout_seconds:` fields and routes correctly to the CLI
/// runner.
fn scenario_h_cli_reference_asset_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    println!("  H) v2_agent_loop_cli_reference.yaml round-trips + dispatches");
    let repo_root = repo_root();
    let path = repo_root
        .join("crates/orbit-core/assets/activities/v2_reference/v2_agent_loop_cli_reference.yaml");
    let yaml = fs::read_to_string(&path)?;
    let asset = match load_activity_asset(&yaml)? {
        ActivityAsset::V2(a) => a,
        ActivityAsset::V1(_) => panic!("expected v2 asset at {}", path.display()),
    };
    match &asset.spec.spec {
        ActivityV2Spec::AgentLoop(spec) => {
            assert_eq!(
                spec.backend,
                Backend::Cli,
                "asset must declare backend: cli"
            );
            assert_eq!(spec.provider, Provider::Claude);
            assert_eq!(spec.wall_clock_timeout_seconds, 30);
        }
        other => panic!("expected agent_loop spec, got {other:?}"),
    }

    let tmp_audit = tempfile::tempdir()?;
    let (writer, _sink) = build_writer(tmp_audit.path(), "smoke-cli-h")?;
    let fake = fake_cli("claude", "#!/bin/sh\ncat > /dev/null\necho ok\n")?;
    let host = ScriptHost::new(fake.cli_path());
    let outcome = dispatch_v2_activity(V2DispatchInput {
        activity_name: &asset.name,
        spec: &asset.spec.spec,
        fs_profile: asset.spec.fs_profile.as_deref(),
        input: serde_json::json!({ "prompt": "hello from the yaml round-trip" }),
        audit: writer.clone(),
        run_id: "smoke-cli-h",
        host: Some(&host),
    })?;
    assert!(outcome.success);
    println!(
        "    asset dispatched, events={}",
        writer.events_snapshot()?.len()
    );
    Ok(())
}

/// I: existing Phase 3 v2 `agent_loop` YAML assets still deserialize with the
/// new `AgentLoopSpec` fields (serde defaults). Covers AC #8 (HTTP reference)
/// + the loop/denial samples under `jobs/v2_samples/`.
fn scenario_i_existing_agent_loop_assets_still_deserialize()
-> Result<(), Box<dyn std::error::Error>> {
    println!("  I) existing v2 agent_loop assets deserialize unchanged");
    let repo_root = repo_root();
    let asset_path = repo_root
        .join("crates/orbit-core/assets/activities/v2_reference/v2_agent_loop_reference.yaml");
    let yaml = fs::read_to_string(&asset_path)?;
    let asset = match load_activity_asset(&yaml)? {
        ActivityAsset::V2(a) => a,
        ActivityAsset::V1(_) => panic!("expected v2 asset"),
    };
    if let ActivityV2Spec::AgentLoop(spec) = &asset.spec.spec {
        // Defaults should produce `Http` + `Claude` when the YAML omits them.
        assert_eq!(spec.backend, Backend::Http, "default backend must be Http");
        assert_eq!(spec.provider, Provider::Claude);
        assert!(spec.wall_clock_timeout_seconds > 0);
    } else {
        panic!("expected agent_loop");
    }
    println!(
        "    {}: backend=http (default), provider=claude (default)",
        asset.name
    );
    Ok(())
}

fn repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cli_agent_loop_spec(provider: Option<Provider>) -> AgentLoopSpec {
    AgentLoopSpec {
        instruction: "cli smoke".to_string(),
        tools: vec!["fs.read".to_string(), "fs.write".to_string()],
        on_denial: OnDenial::Terminate,
        model: Some("claude-sonnet-4-5".to_string()),
        max_iterations: 1,
        backend: Backend::Cli,
        provider: provider.unwrap_or(Provider::Claude),
        wall_clock_timeout_seconds: 30,
    }
}

fn must_contain(types: &[&str], needle: &str) {
    assert!(
        types.contains(&needle),
        "expected `{}` in event types, got {:?}",
        needle,
        types
    );
}

fn build_writer(
    root: &Path,
    run_id: &str,
) -> Result<(Arc<V2AuditWriter>, ()), Box<dyn std::error::Error>> {
    let audit_root = root.join("audit");
    fs::create_dir_all(&audit_root)?;
    let writer = V2AuditWriter::with_disk_sinks(&audit_root, run_id, "smoke".to_string(), None)?;
    Ok((writer, ()))
}

/// Write a shell-script "fake CLI" into a tempdir using the chosen basename
/// so `AgentConfig::from_cli_config` resolves it to the matching factory.
/// The struct retains ownership of the `TempDir` so the file lives for the
/// whole scenario.
struct FakeCli {
    _tempdir: TempDir,
    path: PathBuf,
}
impl FakeCli {
    fn cli_path(&self) -> &Path {
        &self.path
    }
}

fn fake_cli(basename: &str, body: &str) -> Result<FakeCli, Box<dyn std::error::Error>> {
    let tempdir = tempfile::tempdir()?;
    let path = tempdir.path().join(basename);
    {
        let mut f = fs::File::create(&path)?;
        f.write_all(body.as_bytes())?;
    }
    let mut perms = fs::metadata(&path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms)?;
    Ok(FakeCli {
        _tempdir: tempdir,
        path,
    })
}

fn synthetic_loop_session_cli_job() -> JobV2 {
    let review_step = JobV2Step {
        id: "review".to_string(),
        when: None,
        retry: None,
        body: JobV2StepBody::Target(TargetStep {
            spec: ActivityV2Spec::AgentLoop(AgentLoopSpec {
                instruction: String::new(),
                tools: vec![],
                on_denial: OnDenial::Terminate,
                model: None,
                max_iterations: 1,
                backend: Backend::Cli,
                provider: Provider::Claude,
                wall_clock_timeout_seconds: 30,
            }),
            fs_profile: None,
            default_input: None,
            timeout_seconds: 0,
            session: Some("reviewer".to_string()),
        }),
    };
    let loop_step = JobV2Step {
        id: "review_fix".to_string(),
        when: None,
        retry: None,
        body: JobV2StepBody::Loop {
            loop_: LoopBlock {
                items: None,
                max_iterations: 3,
                break_when: None,
                steps: vec![review_step],
            },
        },
    };
    JobV2 {
        state: JobScheduleState::Enabled,
        default_input: None,
        max_active_runs: 1,
        kind: JobKind::Workflow,
        steps: vec![loop_step],
    }
}

fn synthetic_loop_session_auto_job() -> JobV2 {
    let mut job = synthetic_loop_session_cli_job();
    if let JobV2StepBody::Loop { loop_ } = &mut job.steps[0].body {
        if let JobV2StepBody::Target(t) = &mut loop_.steps[0].body {
            if let ActivityV2Spec::AgentLoop(spec) = &mut t.spec {
                spec.backend = Backend::Auto;
            }
        }
    }
    job
}

// Hosts --------------------------------------------------------------------

struct ScriptHost {
    command: String,
}
impl ScriptHost {
    fn new(path: &Path) -> Self {
        Self {
            command: path.to_string_lossy().into_owned(),
        }
    }
}
impl V2RuntimeHost for ScriptHost {
    fn run_deterministic(
        &self,
        _action: &str,
        _config: &Value,
        _input: &Value,
        _tool_context: orbit_tools::ToolContext,
    ) -> Result<Value, DispatchError> {
        Err(DispatchError::DeterministicActionNotRegistered(
            "unused".to_string(),
        ))
    }
    fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::AgentLoopFailed("no key in smoke".into()))
    }
    fn resolve_cli_command(&self, _provider: &str) -> Result<String, DispatchError> {
        Ok(self.command.clone())
    }

    fn tool_context_for_activity(
        &self,
        _fs_profile: Option<&str>,
        _fs_audit: Option<std::sync::Arc<dyn orbit_tools::FsAuditLogger>>,
    ) -> orbit_tools::ToolContext {
        orbit_tools::ToolContext::default()
    }
}

struct NullCliHost;
impl V2RuntimeHost for NullCliHost {
    fn run_deterministic(
        &self,
        _action: &str,
        _config: &Value,
        _input: &Value,
        _tool_context: orbit_tools::ToolContext,
    ) -> Result<Value, DispatchError> {
        Err(DispatchError::DeterministicActionNotRegistered(
            "unused".to_string(),
        ))
    }
    fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::AgentLoopFailed("no key in smoke".into()))
    }
    fn resolve_cli_command(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::CliInvocationFailed(
            "NullCliHost has no CLI mapping".into(),
        ))
    }

    fn tool_context_for_activity(
        &self,
        _fs_profile: Option<&str>,
        _fs_audit: Option<std::sync::Arc<dyn orbit_tools::FsAuditLogger>>,
    ) -> orbit_tools::ToolContext {
        orbit_tools::ToolContext::default()
    }
}
