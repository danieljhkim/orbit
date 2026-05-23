use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs2::FileExt;
use orbit_common::types::{
    AuditEventStatus, LearningInjectionCaps, LearningInjectionState, LearningReminder, OrbitError,
};
use orbit_store::{AuditEventInsertParams, LearningSearchParams};
use serde_json::Value;
use serde_json::json;

use crate::OrbitRuntime;
use crate::redact_sensitive_env_text;

const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(5);
const LOCK_RETRY_BUDGET: Duration = Duration::from_millis(50);

pub const ORBIT_BIN_ENV: &str = "ORBIT_BIN";
pub const ORBIT_SESSION_ID_ENV: &str = "ORBIT_SESSION_ID";
pub const ORBIT_LEARNING_PER_CALL_CAP_ENV: &str = "ORBIT_LEARNING_PER_CALL_CAP";
pub const ORBIT_LEARNING_SESSION_CAP_ENV: &str = "ORBIT_LEARNING_SESSION_CAP";

pub type SessionLearningState = LearningInjectionState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookPayload {
    pub tool_name: String,
    pub target_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookOutputFormat {
    Claude,
    Codex,
    Gemini,
    Grok,
}

pub const CLAUDE_PRETOOLUSE_TOOLS: &[&str] = &["Edit", "Write", "Read"];
pub const CODEX_PRETOOLUSE_TOOLS: &[&str] = &["Bash", "apply_patch", "mcp"];
pub const GEMINI_PRETOOLUSE_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "edit",
    "replace",
    "run_shell_command",
    "Read",
    "Write",
    "Edit",
    "Bash",
];

pub fn run_pretooluse(runtime: &OrbitRuntime, format: HookOutputFormat) -> Option<String> {
    let start = Instant::now();
    match run_pretooluse_payload(runtime, format, start) {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(error = %redact_sensitive_env_text(&error), "learning hook failed open");
            None
        }
    }
}

pub fn render_reminders(
    format: HookOutputFormat,
    admitted: &[LearningReminder],
) -> Result<String, OrbitError> {
    match format {
        HookOutputFormat::Claude | HookOutputFormat::Grok => Ok(render_claude(admitted)),
        HookOutputFormat::Codex => render_codex(admitted),
        HookOutputFormat::Gemini => render_gemini(admitted),
    }
}

pub fn render_claude(admitted: &[LearningReminder]) -> String {
    orbit_common::types::render_reminder_block(admitted)
}

pub fn render_codex(admitted: &[LearningReminder]) -> Result<String, OrbitError> {
    render_json_context("PreToolUse", admitted)
}

pub fn render_gemini(admitted: &[LearningReminder]) -> Result<String, OrbitError> {
    // Gemini CLI names its documented pre-tool hook event `BeforeTool`; the
    // renderer stays separate so the wiring can change when Gemini's hook
    // context surface settles.
    render_json_context("BeforeTool", admitted)
}

pub fn parse_payload(stdin: &str) -> Option<HookPayload> {
    parse_payload_with_tools(stdin, CLAUDE_PRETOOLUSE_TOOLS)
}

pub fn parse_payload_with_tools(stdin: &str, accepted_tools: &[&str]) -> Option<HookPayload> {
    let value: Value = serde_json::from_str(stdin.trim()).ok()?;
    let object = value.as_object()?;
    let tool_name = string_field(&value, &["tool_name", "toolName"])?;
    if !tool_name_allowed(tool_name, accepted_tools) {
        return None;
    }

    let tool_input = object
        .get("tool_input")
        .or_else(|| object.get("toolInput"))
        .and_then(Value::as_object);
    let target_path = tool_input
        .and_then(first_path_in_object)
        .or_else(|| first_path_in_value(&value))
        .or_else(|| {
            tool_input
                .and_then(|input| {
                    ["patch", "diff"]
                        .iter()
                        .find_map(|key| input.get(*key).and_then(trimmed_string))
                })
                .and_then(path_from_patch)
        })
        .or_else(|| {
            tool_input
                .and_then(|input| {
                    ["command", "cmd"]
                        .iter()
                        .find_map(|key| input.get(*key).and_then(trimmed_string))
                })
                .and_then(path_from_shell_command)
        })?;

    Some(HookPayload {
        tool_name: tool_name.to_string(),
        target_path: target_path.to_string(),
    })
}

pub fn caps_from_env() -> LearningInjectionCaps {
    LearningInjectionCaps {
        per_call: cap_from_env(
            ORBIT_LEARNING_PER_CALL_CAP_ENV,
            orbit_common::types::DEFAULT_LEARNING_REMINDER_PER_CALL_CAP,
        ),
        per_session_hard: cap_from_env(
            ORBIT_LEARNING_SESSION_CAP_ENV,
            orbit_common::types::DEFAULT_LEARNING_REMINDER_SESSION_CAP,
        ),
    }
}

pub fn state_file_path(
    repo_root: &Path,
    session_id: Option<&str>,
    tmpdir: &Path,
    ppid: u32,
) -> PathBuf {
    match session_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(session_id) => repo_root
            .join(".orbit")
            .join("state")
            .join("sessions")
            .join(session_id)
            .join("learnings.json"),
        None => tmpdir.join(format!("orbit-learning-hook-{ppid}.json")),
    }
}

pub fn parse_state_json(raw: &str) -> SessionLearningState {
    let Ok(value) = serde_json::from_str::<Value>(raw.trim()) else {
        return SessionLearningState::new();
    };
    let Some(object) = value.as_object() else {
        return SessionLearningState::new();
    };
    let emitted_ids = object
        .get("emitted_ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(ToOwned::to_owned))
        .collect::<std::collections::BTreeSet<_>>();
    let count = object
        .get("count")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(emitted_ids.len());
    SessionLearningState { emitted_ids, count }
}

pub fn merge_state(
    mut prior: SessionLearningState,
    candidates: &[LearningReminder],
    caps: LearningInjectionCaps,
) -> (SessionLearningState, Vec<LearningReminder>) {
    let admitted = prior.admit_reminders(candidates, caps);
    (prior, admitted)
}

pub fn reminders_from_search_results(
    results: Vec<orbit_store::LearningSearchResult>,
) -> Vec<LearningReminder> {
    results
        .into_iter()
        .map(|result| LearningReminder {
            id: result.learning.id,
            summary: result.learning.summary,
            comments: Vec::new(),
        })
        .collect()
}

pub(crate) fn update_state_file(
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

impl OrbitRuntime {
    pub fn learning_hook_target_is_searchable(
        &self,
        target_path: &str,
    ) -> Result<bool, OrbitError> {
        let normalized = target_path.trim().replace('\\', "/");
        if normalized == "~" || normalized.starts_with("~/") {
            return Ok(false);
        }

        crate::command::learning::learning_search_path_matches_workspace(
            &self.paths().repo_root,
            target_path,
        )
    }

    pub fn learning_hook_state_file_path(
        &self,
        session_id: Option<&str>,
        tmpdir: &Path,
        ppid: u32,
    ) -> PathBuf {
        state_file_path(&self.paths().repo_root, session_id, tmpdir, ppid)
    }
}

fn run_pretooluse_payload(
    runtime: &OrbitRuntime,
    format: HookOutputFormat,
    start: Instant,
) -> Result<Option<String>, String> {
    let mut stdin = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin)
        .map_err(|error| format!("read stdin: {error}"))?;

    run_pretooluse_input(runtime, &stdin, format, start)
}

pub(crate) fn run_pretooluse_input(
    runtime: &OrbitRuntime,
    stdin: &str,
    format: HookOutputFormat,
    start: Instant,
) -> Result<Option<String>, String> {
    let Some(payload) = parse_payload_with_tools(stdin, accepted_tools(format)) else {
        return Ok(None);
    };

    let caps = caps_from_env();
    if !runtime
        .learning_hook_target_is_searchable(&payload.target_path)
        .map_err(|error| format!("classify learning target path: {error}"))?
    {
        return Ok(None);
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
        return Ok(None);
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
        return Ok(None);
    }

    emit_learning_injected_audit(
        runtime,
        &payload.tool_name,
        &payload.target_path,
        session_id.as_deref(),
        &admitted,
        start.elapsed(),
    )?;

    render_reminders(format, &admitted)
        .map(Some)
        .map_err(|error| format!("render reminders: {error}"))
}

pub(crate) fn accepted_tools(format: HookOutputFormat) -> &'static [&'static str] {
    match format {
        HookOutputFormat::Claude | HookOutputFormat::Grok => CLAUDE_PRETOOLUSE_TOOLS,
        HookOutputFormat::Codex => CODEX_PRETOOLUSE_TOOLS,
        HookOutputFormat::Gemini => GEMINI_PRETOOLUSE_TOOLS,
    }
}

pub(crate) fn learning_hook_tmpdir() -> PathBuf {
    std::env::var("TMPDIR")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn render_json_context(
    event_name: &str,
    admitted: &[LearningReminder],
) -> Result<String, OrbitError> {
    let block = render_claude(admitted);
    serde_json::to_string(&json!({
        "hookSpecificOutput": {
            "hookEventName": event_name,
            "additionalContext": block,
        }
    }))
    .map_err(|error| OrbitError::Execution(format!("serialize hook output: {error}")))
}

fn tool_name_allowed(tool_name: &str, accepted_tools: &[&str]) -> bool {
    accepted_tools.iter().any(|accepted| {
        tool_name == *accepted
            || (*accepted == "mcp" && tool_name.starts_with("mcp__"))
            || (*accepted == "mcp" && tool_name.starts_with("mcp."))
    })
}

fn first_path_in_value(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    first_path_in_object(object)
}

fn first_path_in_object(object: &serde_json::Map<String, Value>) -> Option<&str> {
    const STRING_KEYS: &[&str] = &[
        "file_path",
        "filePath",
        "path",
        "absolute_file_path",
        "absoluteFilePath",
        "fileName",
        "filename",
        "name",
    ];
    const ARRAY_KEYS: &[&str] = &[
        "file_paths",
        "filePaths",
        "paths",
        "files",
        "fileNames",
        "filenames",
        "absolute_file_paths",
        "absoluteFilePaths",
    ];

    STRING_KEYS
        .iter()
        .find_map(|key| object.get(*key).and_then(trimmed_string))
        .or_else(|| {
            ARRAY_KEYS.iter().find_map(|key| {
                object
                    .get(*key)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .find_map(trimmed_string)
            })
        })
}

fn string_field<'a>(value: &'a Value, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(trimmed_string))
}

fn trimmed_string(value: &Value) -> Option<&str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn path_from_patch(patch: &str) -> Option<&str> {
    patch.lines().find_map(|line| {
        let line = line.trim();
        [
            "*** Update File: ",
            "*** Add File: ",
            "*** Delete File: ",
            "*** Move to: ",
        ]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix).map(str::trim))
        .filter(|value| !value.is_empty())
    })
}

fn path_from_shell_command(command: &str) -> Option<&str> {
    command
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            })
        })
        .filter(|token| !token.is_empty())
        .find(|token| looks_like_path(token))
}

fn looks_like_path(token: &str) -> bool {
    if token.starts_with('-') || matches!(token, "|" | ">" | "<" | "&&" | "||") {
        return false;
    }
    token.contains('/')
        || token.starts_with('.')
        || [
            ".rs", ".toml", ".json", ".md", ".yaml", ".yml", ".txt", ".sh", ".py",
        ]
        .iter()
        .any(|suffix| token.ends_with(suffix))
}

fn cap_from_env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.max(1))
        .unwrap_or(default)
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
