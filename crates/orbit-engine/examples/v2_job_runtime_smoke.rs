//! Phase 3 end-to-end smoke for the v2 job DAG executor.
//!
//! Runs each sample under `crates/orbit-core/assets/jobs/v2_samples/` through
//! `orbit_engine::v2::execute_job` and asserts the expected §7 envelope
//! events appear (or — for the denial sample — don't appear).
//!
//! No credentials needed: shell samples exec real `sh`; the loop + denial
//! samples use the replay path (`ORBIT_V2_REPLAY{,_FIXTURE}`).
//!
//! Usage:
//!     cargo run -p orbit-engine --example v2_job_runtime_smoke

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use orbit_agent::loop_engine::{InMemorySink, LoopAuditEvent};
use orbit_engine::v2::{
    DispatchError, V2AuditWriter, V2JsonlSink, V2RuntimeHost, execute_job, reset_replay_transport,
};
use orbit_types::v2::{JobAsset, V2AuditEventKind, load_job_asset};
use serde_json::Value;

fn main() -> ExitCode {
    let samples_dir = workspace_root().join("crates/orbit-core/assets/jobs/v2_samples");
    let fixtures_dir = samples_dir.join("fixtures");
    let tmp_audit_root = std::env::temp_dir().join("orbit-v2-job-smoke");
    let _ = std::fs::create_dir_all(&tmp_audit_root);

    let mut results: Vec<(String, Result<(), String>)> = Vec::new();

    results.push((
        "parallel_branches".into(),
        smoke_parallel_success(&samples_dir, &tmp_audit_root),
    ));
    results.push((
        "parallel_branches_failing".into(),
        smoke_parallel_failing(&samples_dir, &tmp_audit_root),
    ));
    results.push((
        "conditional_on_value (approved)".into(),
        smoke_conditional_approved(&samples_dir, &tmp_audit_root),
    ));
    results.push((
        "conditional_on_value (rejected)".into(),
        smoke_conditional_rejected(&samples_dir, &tmp_audit_root),
    ));
    results.push((
        "retry_backoff".into(),
        smoke_retry(&samples_dir, &tmp_audit_root),
    ));
    results.push((
        "fanout_fanin".into(),
        smoke_fanout(&samples_dir, &tmp_audit_root),
    ));
    results.push((
        "loop_review_fix".into(),
        smoke_loop_converge(&samples_dir, &fixtures_dir, &tmp_audit_root),
    ));
    results.push((
        "loop_no_converge".into(),
        smoke_loop_diverge(&samples_dir, &tmp_audit_root),
    ));
    results.push((
        "tool_denial_no_retry".into(),
        smoke_denial_no_retry(&samples_dir, &tmp_audit_root),
    ));

    let mut ok = 0;
    let mut fail = 0;
    for (name, res) in &results {
        match res {
            Ok(()) => {
                println!("[PASS] {name}");
                ok += 1;
            }
            Err(err) => {
                println!("[FAIL] {name}: {err}");
                fail += 1;
            }
        }
    }
    println!("\n{ok} passed, {fail} failed");
    if fail == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

// ---------------------------------------------------------------------------
// Smokes
// ---------------------------------------------------------------------------

fn smoke_parallel_success(samples: &Path, audit_root: &Path) -> Result<(), String> {
    let events = run_sample(
        samples.join("parallel_branches.yaml"),
        audit_root,
        "parallel-ok",
        Value::Null,
        true,
        &[],
    )?;
    require_event_type(&events, "step.join")?;
    let join = find_event(&events, "step.join").unwrap();
    if let V2AuditEventKind::StepJoin {
        branch_outcomes, ..
    } = &join.kind
    {
        if branch_outcomes.len() != 3 {
            return Err(format!(
                "expected 3 branch outcomes, got {}",
                branch_outcomes.len()
            ));
        }
        if !branch_outcomes.iter().all(|b| b.outcome == "success") {
            return Err("expected all branches to succeed".into());
        }
    }
    Ok(())
}

fn smoke_parallel_failing(samples: &Path, audit_root: &Path) -> Result<(), String> {
    let events = run_sample(
        samples.join("parallel_branches_failing.yaml"),
        audit_root,
        "parallel-fail",
        Value::Null,
        false,
        &[],
    )?;
    require_event_type(&events, "step.join")?;
    let join = find_event(&events, "step.join").unwrap();
    if let V2AuditEventKind::StepJoin {
        branch_outcomes, ..
    } = &join.kind
    {
        if !branch_outcomes.iter().any(|b| b.outcome == "failed") {
            return Err("expected at least one branch to fail".into());
        }
    }
    Ok(())
}

fn smoke_conditional_approved(samples: &Path, audit_root: &Path) -> Result<(), String> {
    let events = run_sample(
        samples.join("conditional_on_value.yaml"),
        audit_root,
        "cond-approved",
        serde_json::json!({"decision": "approved"}),
        true,
        &[],
    )?;
    let skipped = events
        .iter()
        .filter(|e| e.envelope.event_type == "step.skipped")
        .count();
    if skipped != 0 {
        return Err(format!(
            "approved path should not skip; got {skipped} skipped events"
        ));
    }
    Ok(())
}

fn smoke_conditional_rejected(samples: &Path, audit_root: &Path) -> Result<(), String> {
    let events = run_sample(
        samples.join("conditional_on_value.yaml"),
        audit_root,
        "cond-rejected",
        serde_json::json!({"decision": "rejected"}),
        true,
        &[],
    )?;
    let skipped_merge = events.iter().any(|e| {
        e.envelope.event_type == "step.skipped"
            && matches!(&e.kind, V2AuditEventKind::StepSkipped { step_id, .. } if step_id == "merge")
    });
    if !skipped_merge {
        return Err("expected `merge` step to emit step.skipped under rejected".into());
    }
    Ok(())
}

fn smoke_retry(samples: &Path, audit_root: &Path) -> Result<(), String> {
    // Ensure any leftover markers from previous runs don't skew behavior.
    let _ = std::fs::remove_file("/tmp/orbit_retry_flake");
    let _ = std::fs::remove_file("/tmp/orbit_retry_flake.2");
    let events = run_sample(
        samples.join("retry_backoff.yaml"),
        audit_root,
        "retry",
        Value::Null,
        true,
        &[],
    )?;
    let retries: Vec<_> = events
        .iter()
        .filter(|e| e.envelope.event_type == "step.retry")
        .collect();
    if retries.len() < 2 {
        return Err(format!(
            "expected >=2 step.retry events, got {}",
            retries.len()
        ));
    }
    // AC #3: "doubling-then-capping behavior verified by inspecting the
    // timestamp delta between attempts against the declared cap." The sample
    // declares initial=100ms, cap=500ms, strategy=exponential — so the two
    // recorded backoffs should be 100ms and 200ms (neither hitting the cap).
    for (i, retry) in retries.iter().enumerate().take(2) {
        if let V2AuditEventKind::StepRetry {
            next_backoff_ms, ..
        } = &retry.kind
        {
            let expected = 100u64 << i; // 100, 200, 400, ... capped at 500
            let expected = expected.min(500);
            if *next_backoff_ms != expected {
                return Err(format!(
                    "step.retry[{i}] next_backoff_ms={next_backoff_ms}, expected={expected}"
                ));
            }
        }
    }
    // And the observed timestamp delta between the two step.retry events must
    // be at least the first declared backoff (100ms) — tolerates scheduling
    // jitter, rejects "didn't sleep at all".
    if let (V2AuditEventKind::StepRetry { .. }, V2AuditEventKind::StepRetry { .. }) =
        (&retries[0].kind, &retries[1].kind)
    {
        let t0 = retries[0].envelope.ts;
        let t1 = retries[1].envelope.ts;
        let delta = (t1 - t0).num_milliseconds();
        if delta < 100 {
            return Err(format!(
                "step.retry timestamps too close: {delta}ms (expected >=100ms for exponential backoff)"
            ));
        }
    }
    Ok(())
}

fn smoke_fanout(samples: &Path, audit_root: &Path) -> Result<(), String> {
    let events = run_sample(
        samples.join("fanout_fanin.yaml"),
        audit_root,
        "fanout",
        serde_json::json!({"items": ["a", "b", "c"]}),
        true,
        &[],
    )?;
    let fanout = find_event(&events, "fanout.dispatched")
        .ok_or_else(|| "no fanout.dispatched event".to_string())?;
    if let V2AuditEventKind::FanoutDispatched { worker_count, .. } = &fanout.kind {
        if *worker_count != 3 {
            return Err(format!("expected worker_count=3, got {worker_count}"));
        }
    }
    let fanin =
        find_event(&events, "fanin.joined").ok_or_else(|| "no fanin.joined event".to_string())?;
    if let V2AuditEventKind::FaninJoined {
        collected, failed, ..
    } = &fanin.kind
    {
        if *collected != 3 || *failed != 0 {
            return Err(format!(
                "expected collected=3 failed=0, got collected={collected} failed={failed}"
            ));
        }
    }
    let worker_events = events
        .iter()
        .filter(|e| e.envelope.event_type == "worker.state")
        .count();
    if worker_events < 6 {
        return Err(format!(
            "expected >=6 worker.state events (3 workers * 2 states), got {worker_events}"
        ));
    }
    Ok(())
}

fn smoke_loop_converge(samples: &Path, fixtures: &Path, audit_root: &Path) -> Result<(), String> {
    let fixture_path = fixtures.join("loop_review_fix.json");
    reset_replay_transport();
    unsafe {
        env::set_var(
            "ORBIT_V2_REPLAY_FIXTURE",
            fixture_path.display().to_string(),
        );
    }

    let events_res = run_sample(
        samples.join("loop_review_fix.yaml"),
        audit_root,
        "loop-converge",
        Value::Null,
        true,
        &[],
    );

    let (events, loop_events) = match events_res {
        Ok(events) => {
            let loop_events = take_last_loop_events();
            (events, loop_events)
        }
        Err(err) => {
            unsafe {
                env::remove_var("ORBIT_V2_REPLAY_FIXTURE");
            }
            return Err(err);
        }
    };
    unsafe {
        env::remove_var("ORBIT_V2_REPLAY_FIXTURE");
    }

    let iter_starts: Vec<_> = events
        .iter()
        .filter(|e| e.envelope.event_type == "loop.iteration.start")
        .collect();
    if iter_starts.len() < 2 {
        return Err(format!(
            "expected >=2 loop.iteration.start events, got {}",
            iter_starts.len()
        ));
    }
    let iter_ends: Vec<_> = events
        .iter()
        .filter(|e| e.envelope.event_type == "loop.iteration.end")
        .collect();
    let broke = iter_ends.iter().any(|e| {
        matches!(
            &e.kind,
            V2AuditEventKind::LoopIterationEnd { broke, .. } if *broke
        )
    });
    if !broke {
        return Err("no loop.iteration.end reported broke=true".into());
    }

    // Session persistence: every tool.call.* from loop_events should share
    // the same session_id across iterations.
    let session_ids: Vec<String> = loop_events
        .iter()
        .filter_map(|e| match e {
            LoopAuditEvent::ToolCallRequested { session_id, .. } => Some(session_id.clone()),
            LoopAuditEvent::ToolCallResult { session_id, .. } => Some(session_id.clone()),
            _ => None,
        })
        .collect();
    if session_ids.len() < 2 {
        return Err(format!(
            "expected >=2 tool.call.* events (one per iteration), got {}",
            session_ids.len()
        ));
    }
    let first = &session_ids[0];
    if !session_ids.iter().all(|s| s == first) {
        return Err(format!(
            "session_id varies across iterations: {session_ids:?}"
        ));
    }
    Ok(())
}

fn smoke_loop_diverge(samples: &Path, audit_root: &Path) -> Result<(), String> {
    let events = run_sample(
        samples.join("loop_no_converge.yaml"),
        audit_root,
        "loop-diverge",
        Value::Null,
        true,
        &[],
    )?;
    if find_event(&events, "loop.did_not_converge").is_none() {
        return Err("no loop.did_not_converge event".into());
    }
    Ok(())
}

fn smoke_denial_no_retry(samples: &Path, audit_root: &Path) -> Result<(), String> {
    reset_replay_transport();
    unsafe {
        env::set_var("ORBIT_V2_REPLAY", "tool_denial");
    }
    let events_res = run_sample(
        samples.join("tool_denial_no_retry.yaml"),
        audit_root,
        "denial",
        Value::Null,
        false, // the job returns false because ToolDenied propagates as error
        &[],
    );
    unsafe {
        env::remove_var("ORBIT_V2_REPLAY");
    }

    // run_sample returns Err when the job call errors; in this sample the
    // error IS the expected outcome. Inspect the persisted audit writer.
    let events = match events_res {
        Ok(events) => events,
        Err(err) if err.contains("execute_job errored: tool") => take_last_events(),
        Err(err) => return Err(err),
    };

    if find_event(&events, "tool.denied").is_none() {
        return Err("no tool.denied event emitted".into());
    }
    let retry_for_step = events.iter().any(|e| {
        e.envelope.event_type == "step.retry"
            && matches!(&e.kind, V2AuditEventKind::StepRetry { step_id, .. } if step_id == "denied_call")
    });
    if retry_for_step {
        return Err("step.retry emitted for `denied_call` — denial must skip retry".into());
    }
    let denied = events.iter().any(|e| {
        e.envelope.event_type == "step.denied"
            && matches!(&e.kind, V2AuditEventKind::StepDenied { step_id, .. } if step_id == "denied_call")
    });
    if !denied {
        return Err("no step.denied event for `denied_call`".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Run a sample through `execute_job`, return the emitted envelope events,
/// and stash the underlying loop-engine events for callers that inspect them.
fn run_sample(
    path: PathBuf,
    audit_root: &Path,
    run_id: &str,
    input: Value,
    expect_success: bool,
    _filters: &[&str],
) -> Result<Vec<orbit_types::v2::V2AuditEvent>, String> {
    let yaml = std::fs::read_to_string(&path).map_err(|e| format!("read {path:?}: {e}"))?;
    let spec = match load_job_asset(&yaml).map_err(|e| format!("load {path:?}: {e}"))? {
        JobAsset::V2(a) => a.spec,
        JobAsset::V1(_) => return Err("parsed as v1, expected v2".into()),
    };

    let blob_dir = audit_root.join("blobs");
    let _ = std::fs::create_dir_all(&blob_dir);
    let inner = Arc::new(InMemorySink::new(blob_dir));
    let envelope =
        Arc::new(V2JsonlSink::open(audit_root, run_id).map_err(|e| format!("sinks: {e}"))?);
    let writer = Arc::new(
        V2AuditWriter::new(run_id, "smoke-agent", inner.clone() as Arc<_>)
            .with_envelope_sink(envelope),
    );

    let result = execute_job(&spec, input, run_id, writer.clone(), &StubHost);

    // Stash loop-engine events for loop/denial smokes.
    *LAST_LOOP.lock().unwrap() = inner.events();

    let events = writer
        .events_snapshot()
        .map_err(|e| format!("snapshot: {e:?}"))?;
    stash_envelope_events(events.clone());

    match result {
        Ok(outcome) => {
            if expect_success && !outcome.success {
                return Err(format!("expected success, got success={}", outcome.success));
            }
            if !expect_success && outcome.success {
                return Err("expected non-success, got success=true".into());
            }
            Ok(events)
        }
        Err(err) => Err(format!("execute_job errored: {err}")),
    }
}

fn find_event<'a>(
    events: &'a [orbit_types::v2::V2AuditEvent],
    event_type: &str,
) -> Option<&'a orbit_types::v2::V2AuditEvent> {
    events.iter().find(|e| e.envelope.event_type == event_type)
}

fn require_event_type(
    events: &[orbit_types::v2::V2AuditEvent],
    event_type: &str,
) -> Result<(), String> {
    if find_event(events, event_type).is_none() {
        return Err(format!("missing {event_type}"));
    }
    Ok(())
}

// Last-seen-buffers so the denial smoke can inspect events after a failed
// `execute_job` call bubbles up as Err().
use std::sync::Mutex;
static LAST_ENVELOPE: Mutex<Vec<orbit_types::v2::V2AuditEvent>> = Mutex::new(Vec::new());
static LAST_LOOP: Mutex<Vec<LoopAuditEvent>> = Mutex::new(Vec::new());

fn stash_envelope_events(events: Vec<orbit_types::v2::V2AuditEvent>) {
    *LAST_ENVELOPE.lock().unwrap() = events;
}

fn take_last_events() -> Vec<orbit_types::v2::V2AuditEvent> {
    LAST_ENVELOPE.lock().unwrap().clone()
}

fn take_last_loop_events() -> Vec<LoopAuditEvent> {
    LAST_LOOP.lock().unwrap().clone()
}

// ---------------------------------------------------------------------------
// Stub host
// ---------------------------------------------------------------------------

struct StubHost;

impl V2RuntimeHost for StubHost {
    fn run_deterministic(
        &self,
        action: &str,
        _config: &Value,
        _input: &Value,
    ) -> Result<Value, DispatchError> {
        Err(DispatchError::DeterministicActionNotRegistered(
            action.into(),
        ))
    }

    fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::AgentLoopFailed(
            "smoke host: no credentials".into(),
        ))
    }

    fn resolve_cli_command(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::CliInvocationFailed(
            "smoke host: no CLI mapping".into(),
        ))
    }
}

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .to_path_buf()
}
