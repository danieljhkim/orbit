use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs2::FileExt;
use orbit_common::types::{
    AuditEventStatus, ReviewMessage, ReviewThread, ReviewThreadAnchor, ReviewThreadStatus,
    all_agent_families, infer_agent_family_from_model,
};
use orbit_store::AuditEventInsertParams;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::OrbitRuntime;

const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(5);
const LOCK_RETRY_BUDGET: Duration = Duration::from_millis(50);

pub const ORBIT_ACTIVE_TASK_ID_ENV: &str = "ORBIT_ACTIVE_TASK_ID";
pub const ORBIT_TASK_ID_ENV: &str = "ORBIT_TASK_ID";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ReviewThreadHookState {
    #[serde(default)]
    pub tasks: BTreeMap<String, BTreeMap<String, ReviewThreadCursor>>,
}

impl ReviewThreadHookState {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewThreadCursor {
    pub last_seen_message_seq: u64,
    pub status: ReviewThreadStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewThreadReminder {
    pub task_id: String,
    pub thread_id: String,
    pub anchor: ReviewThreadAnchor,
    pub status: ReviewThreadStatus,
    pub last_seen_message_seq: u64,
    pub messages: Vec<ReviewThreadReminderMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewThreadReminderMessage {
    pub seq: u64,
    pub by: String,
    pub author_identity: String,
    pub body: String,
}

pub fn active_task_id_from_env() -> Option<String> {
    // ADR-0182: ORBIT_ACTIVE_TASK_ID is the explicit hook binding; ORBIT_TASK_ID
    // remains a compatibility fallback while older execution paths catch up.
    [ORBIT_ACTIVE_TASK_ID_ENV, ORBIT_TASK_ID_ENV]
        .into_iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
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
            .join("review-threads.json"),
        None => tmpdir.join(format!("orbit-review-thread-hook-{ppid}.json")),
    }
}

pub fn parse_state_json(raw: &str) -> ReviewThreadHookState {
    serde_json::from_str::<ReviewThreadHookState>(raw.trim()).unwrap_or_default()
}

pub fn reminders_from_threads(
    task_id: &str,
    threads: Vec<ReviewThread>,
) -> Vec<ReviewThreadReminder> {
    threads
        .into_iter()
        .filter(|thread| !thread.thread_id.trim().is_empty())
        .map(|thread| {
            let last_seen_message_seq = thread.messages.len() as u64;
            let anchor = thread.anchor();
            ReviewThreadReminder {
                task_id: task_id.to_string(),
                thread_id: thread.thread_id,
                anchor,
                status: thread.status,
                last_seen_message_seq,
                messages: reminder_messages(thread.messages),
            }
        })
        .collect()
}

pub fn merge_state(
    mut prior: ReviewThreadHookState,
    candidates: &[ReviewThreadReminder],
) -> (ReviewThreadHookState, Vec<ReviewThreadReminder>) {
    let mut admitted = Vec::new();
    for candidate in candidates {
        let task_state = prior.tasks.entry(candidate.task_id.clone()).or_default();
        let previous = task_state.get(&candidate.thread_id);
        let previous_seq = previous
            .map(|cursor| cursor.last_seen_message_seq)
            .unwrap_or(0);
        let should_admit = candidate.status == ReviewThreadStatus::Open
            && candidate.last_seen_message_seq > previous_seq;

        if candidate.last_seen_message_seq > previous_seq
            || previous.is_none()
            || previous.is_some_and(|cursor| cursor.status != candidate.status)
        {
            task_state.insert(
                candidate.thread_id.clone(),
                ReviewThreadCursor {
                    last_seen_message_seq: candidate.last_seen_message_seq,
                    status: candidate.status,
                },
            );
        }

        if should_admit {
            admitted.push(candidate.clone());
        }
    }

    (prior, admitted)
}

pub(crate) fn update_state_file(
    state_path: &Path,
    candidates: &[ReviewThreadReminder],
) -> Result<Vec<ReviewThreadReminder>, String> {
    if let Some(parent) = state_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create review-thread state dir {}: {error}",
                parent.display()
            )
        })?;
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(state_path)
        .map_err(|error| {
            format!(
                "open review-thread state file {}: {error}",
                state_path.display()
            )
        })?;

    try_lock_exclusive(&file)?;
    let update_result = update_locked_state(&mut file, candidates);
    let unlock_result = file.unlock().map_err(|error| {
        format!(
            "unlock review-thread state file {}: {error}",
            state_path.display()
        )
    });
    match (update_result, unlock_result) {
        (Ok(admitted), Ok(())) => Ok(admitted),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

pub fn render_review_thread_block(reminders: &[ReviewThreadReminder]) -> String {
    if reminders.is_empty() {
        return String::new();
    }

    let mut out = String::from("<system-reminder>\n");
    out.push_str("Review threads awaiting agent attention:\n\n");
    for reminder in reminders {
        out.push_str(&format!(
            "- Task {} thread {} ({}, last_seen_message_seq={}):\n",
            reminder.task_id,
            reminder.thread_id,
            render_anchor(&reminder.anchor),
            reminder.last_seen_message_seq
        ));
        for message in &reminder.messages {
            out.push_str(&format!(
                "  - message #{} by {} [{}]: {}\n",
                message.seq,
                message.by,
                message.author_identity,
                first_line_or_body(&message.body)
            ));
            for line in message.body.lines().skip(1) {
                out.push_str("    ");
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out.push('\n');
    out.push_str("Action: incorporate trivial asks directly. For non-trivial scope, propose a follow-up with `orbit.task.add` and `relations: [{\"type\":\"spawned_from\",\"target\":\"<task_id>\"}]`.\n");
    out.push_str("Reply on the review thread with the decision, then resolve it by default; a human can re-open by adding a new reply if unsatisfied.\n");
    out.push_str("</system-reminder>");
    out
}

impl OrbitRuntime {
    pub fn review_thread_hook_state_file_path(
        &self,
        session_id: Option<&str>,
        tmpdir: &Path,
        ppid: u32,
    ) -> PathBuf {
        state_file_path(&self.paths().repo_root, session_id, tmpdir, ppid)
    }
}

pub(crate) fn emit_review_thread_surfaced_audit(
    runtime: &OrbitRuntime,
    tool_name: &str,
    target_path: &str,
    session_id: Option<&str>,
    reminder: &ReviewThreadReminder,
    duration: Duration,
) -> Result<(), String> {
    let arguments_json = serde_json::to_string(&json!({
        "task_id": reminder.task_id,
        "thread_id": reminder.thread_id,
        "last_seen_message_seq": reminder.last_seen_message_seq,
        "target_path": target_path,
    }))
    .map_err(|error| format!("serialize review-thread audit arguments: {error}"))?;
    let working_directory = std::env::current_dir()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    let params = AuditEventInsertParams {
        execution_id: orbit_common::types::audit_execution_id("review-thread-hook"),
        command: "hook".to_string(),
        subcommand: Some("pretooluse".to_string()),
        tool_name: Some(tool_name.to_string()),
        target_type: Some("review_thread_surfaced".to_string()),
        target_id: Some(reminder.thread_id.clone()),
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
        task_id: Some(reminder.task_id.clone()),
        job_run_id: std::env::var("ORBIT_RUN_ID")
            .ok()
            .filter(|value| !value.is_empty()),
        activity_id: std::env::var("ORBIT_ACTIVITY_ID")
            .ok()
            .filter(|value| !value.is_empty()),
        step_index: std::env::var("ORBIT_STEP_INDEX")
            .ok()
            .and_then(|value| value.parse().ok()),
        backend: None,
    };

    runtime
        .record_audit_event(&params)
        .map_err(|error| format!("record review-thread audit event: {error}"))
}

fn reminder_messages(messages: Vec<ReviewMessage>) -> Vec<ReviewThreadReminderMessage> {
    messages
        .into_iter()
        .enumerate()
        .map(|(index, message)| ReviewThreadReminderMessage {
            seq: (index + 1) as u64,
            author_identity: author_identity(&message.by),
            by: message.by,
            body: message.body,
        })
        .collect()
}

fn author_identity(by: &str) -> String {
    let label = by.trim();
    if label.eq_ignore_ascii_case("human") {
        return "human".to_string();
    }
    if let Some(family) = infer_agent_family_from_model(label) {
        return format!("agent family {family}");
    }
    if all_agent_families()
        .into_iter()
        .any(|family| label.eq_ignore_ascii_case(family))
    {
        return format!("agent family {}", label.to_ascii_lowercase());
    }
    format!("human ({label})")
}

fn render_anchor(anchor: &ReviewThreadAnchor) -> String {
    match anchor {
        ReviewThreadAnchor::Inline { path, line } => format!("{path}:{line}"),
        ReviewThreadAnchor::TaskLevel => "task-level".to_string(),
    }
}

fn first_line_or_body(body: &str) -> String {
    let mut lines = body.lines();
    let first = lines.next().unwrap_or("").trim();
    if first.is_empty() {
        return "(empty message)".to_string();
    }
    first.to_string()
}

fn try_lock_exclusive(file: &File) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if started.elapsed() >= LOCK_RETRY_BUDGET {
                    return Err("review-thread state file lock timed out".to_string());
                }
                std::thread::sleep(LOCK_RETRY_INTERVAL);
            }
            Err(error) => return Err(format!("lock review-thread state file: {error}")),
        }
    }
}

fn update_locked_state(
    file: &mut File,
    candidates: &[ReviewThreadReminder],
) -> Result<Vec<ReviewThreadReminder>, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("seek review-thread state file: {error}"))?;
    let mut raw = String::new();
    file.read_to_string(&mut raw)
        .map_err(|error| format!("read review-thread state file: {error}"))?;

    let prior = parse_state_json(&raw);
    let (next_state, admitted) = merge_state(prior, candidates);

    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("rewind review-thread state file: {error}"))?;
    file.set_len(0)
        .map_err(|error| format!("truncate review-thread state file: {error}"))?;
    serde_json::to_writer_pretty(&mut *file, &next_state)
        .map_err(|error| format!("serialize review-thread state file: {error}"))?;
    file.write_all(b"\n")
        .map_err(|error| format!("write review-thread state file: {error}"))?;
    file.flush()
        .map_err(|error| format!("flush review-thread state file: {error}"))?;

    Ok(admitted)
}
