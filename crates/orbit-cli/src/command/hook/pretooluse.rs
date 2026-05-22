use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::Args;
use fs2::FileExt;
use orbit_common::types::{AuditEventStatus, LearningInjectionCaps, LearningReminder};
use orbit_core::command::learning_hook::{
    CLAUDE_PRETOOLUSE_TOOLS, CODEX_PRETOOLUSE_TOOLS, GEMINI_PRETOOLUSE_TOOLS, ORBIT_SESSION_ID_ENV,
    caps_from_env, merge_state, parse_payload_with_tools, parse_state_json,
    reminders_from_search_results,
};
use orbit_core::{
    AuditEventInsertParams, LearningSearchParams, OrbitError, OrbitRuntime,
    redact_sensitive_env_text,
};
use serde_json::json;

use crate::command::Execute;
use crate::command::hook::render::{HookOutputFormat, render_reminders};

const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(5);
const LOCK_RETRY_BUDGET: Duration = Duration::from_millis(50);

#[derive(Args)]
pub struct PretooluseArgs {
    /// Render output in the hook format expected by this agent.
    #[arg(long, value_enum, default_value_t = HookOutputFormat::Claude)]
    pub format: HookOutputFormat,
}

impl Execute for PretooluseArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let start = Instant::now();
        if let Err(error) = run_pretooluse(runtime, start, self.format) {
            tracing::warn!(error = %redact_sensitive_env_text(&error), "learning hook failed open");
        }
        Ok(())
    }
}

fn run_pretooluse(
    runtime: &OrbitRuntime,
    start: Instant,
    format: HookOutputFormat,
) -> Result<(), String> {
    let mut stdin = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin)
        .map_err(|error| format!("read stdin: {error}"))?;

    let Some(payload) = parse_payload_with_tools(&stdin, accepted_tools(format)) else {
        return Ok(());
    };

    let caps = caps_from_env();
    if !runtime
        .learning_hook_target_is_searchable(&payload.target_path)
        .map_err(|error| format!("classify learning target path: {error}"))?
    {
        return Ok(());
    }

    let results = runtime
        .search_learnings(LearningSearchParams {
            path: Some(payload.target_path.clone()),
            tag: None,
            query: None,
            limit: Some(caps.per_call),
        })
        .map_err(|error| format!("search learnings: {error}"))?;
    if results.is_empty() {
        return Ok(());
    }

    let candidates = reminders_from_search_results(results);
    let session_id = std::env::var(ORBIT_SESSION_ID_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty());
    let tmpdir = learning_hook_tmpdir();
    let state_path =
        runtime.learning_hook_state_file_path(session_id.as_deref(), &tmpdir, parent_process_id());
    let admitted = update_state_file(&state_path, &candidates, caps)?;
    if admitted.is_empty() {
        return Ok(());
    }

    emit_learning_injected_audit(
        runtime,
        &payload.tool_name,
        &payload.target_path,
        session_id.as_deref(),
        &admitted,
        start.elapsed(),
    )?;

    let output = render_reminders(format, &admitted)
        .map_err(|error| format!("render reminders: {error}"))?;
    println!("{output}");
    Ok(())
}

fn accepted_tools(format: HookOutputFormat) -> &'static [&'static str] {
    match format {
        HookOutputFormat::Claude | HookOutputFormat::Grok => CLAUDE_PRETOOLUSE_TOOLS,
        HookOutputFormat::Codex => CODEX_PRETOOLUSE_TOOLS,
        HookOutputFormat::Gemini => GEMINI_PRETOOLUSE_TOOLS,
    }
}

fn learning_hook_tmpdir() -> PathBuf {
    std::env::var("TMPDIR")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn update_state_file(
    state_path: &Path,
    candidates: &[LearningReminder],
    caps: LearningInjectionCaps,
) -> Result<Vec<LearningReminder>, String> {
    if let Some(parent) = state_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create state dir {}: {error}", parent.display()))?;
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(state_path)
        .map_err(|error| format!("open state file {}: {error}", state_path.display()))?;

    try_lock_exclusive(&file)?;
    let update_result = update_locked_state(&mut file, candidates, caps);
    let unlock_result = file
        .unlock()
        .map_err(|error| format!("unlock state file {}: {error}", state_path.display()));
    match (update_result, unlock_result) {
        (Ok(admitted), Ok(())) => Ok(admitted),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn try_lock_exclusive(file: &File) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if started.elapsed() >= LOCK_RETRY_BUDGET {
                    return Err("state file lock timed out".to_string());
                }
                std::thread::sleep(LOCK_RETRY_INTERVAL);
            }
            Err(error) => return Err(format!("lock state file: {error}")),
        }
    }
}

fn update_locked_state(
    file: &mut File,
    candidates: &[LearningReminder],
    caps: LearningInjectionCaps,
) -> Result<Vec<LearningReminder>, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("seek state file: {error}"))?;
    let mut raw = String::new();
    file.read_to_string(&mut raw)
        .map_err(|error| format!("read state file: {error}"))?;

    let prior = parse_state_json(&raw);
    let (next_state, admitted) = merge_state(prior, candidates, caps);

    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("rewind state file: {error}"))?;
    file.set_len(0)
        .map_err(|error| format!("truncate state file: {error}"))?;
    serde_json::to_writer_pretty(&mut *file, &next_state)
        .map_err(|error| format!("serialize state file: {error}"))?;
    file.write_all(b"\n")
        .map_err(|error| format!("write state file: {error}"))?;
    file.flush()
        .map_err(|error| format!("flush state file: {error}"))?;

    Ok(admitted)
}

fn emit_learning_injected_audit(
    runtime: &OrbitRuntime,
    tool_name: &str,
    target_path: &str,
    session_id: Option<&str>,
    admitted: &[LearningReminder],
    duration: Duration,
) -> Result<(), String> {
    let learning_ids = admitted
        .iter()
        .map(|reminder| reminder.id.clone())
        .collect::<Vec<_>>();
    let arguments_json = serde_json::to_string(&json!({ "learning_ids": learning_ids }))
        .map_err(|error| format!("serialize audit arguments: {error}"))?;
    let working_directory = std::env::current_dir()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    let params = AuditEventInsertParams {
        execution_id: orbit_common::types::audit_execution_id("learning"),
        command: "hook".to_string(),
        subcommand: Some("pretooluse".to_string()),
        tool_name: Some(tool_name.to_string()),
        target_type: Some("learning_injected".to_string()),
        target_id: Some(target_path.to_string()),
        role: "hook".to_string(),
        status: AuditEventStatus::Success,
        exit_code: 0,
        duration_ms: duration.as_millis() as i64,
        working_directory,
        arguments_json: Some(arguments_json),
        stdout_truncated: None,
        stderr_truncated: None,
        error_message: None,
        host: std::env::var("HOSTNAME").ok(),
        pid: std::process::id(),
        session_id: session_id.map(ToOwned::to_owned),
        task_id: std::env::var("ORBIT_TASK_ID")
            .ok()
            .filter(|value| !value.is_empty()),
        job_run_id: std::env::var("ORBIT_RUN_ID")
            .ok()
            .filter(|value| !value.is_empty()),
        activity_id: std::env::var("ORBIT_ACTIVITY_ID")
            .ok()
            .filter(|value| !value.is_empty()),
        step_index: std::env::var("ORBIT_STEP_INDEX")
            .ok()
            .and_then(|value| value.parse().ok()),
    };

    runtime
        .record_audit_event(&params)
        .map_err(|error| format!("record learning audit event: {error}"))
}

#[cfg(unix)]
fn parent_process_id() -> u32 {
    // SAFETY: getppid has no preconditions and only reads process metadata.
    unsafe { libc::getppid() as u32 }
}

#[cfg(not(unix))]
fn parent_process_id() -> u32 {
    std::process::id()
}
