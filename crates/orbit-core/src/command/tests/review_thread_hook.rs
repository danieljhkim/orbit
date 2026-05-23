use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use chrono::Utc;
use orbit_common::types::{ReviewMessage, ReviewThread, ReviewThreadStatus};

use super::super::review_thread_hook::{
    ORBIT_ACTIVE_TASK_ID_ENV, ORBIT_TASK_ID_ENV, ReviewThreadCursor, ReviewThreadHookState,
    active_task_id_from_env, merge_state, parse_state_json, reminders_from_threads,
    render_review_thread_block, state_file_path, update_state_file,
};

#[test]
fn active_task_id_prefers_explicit_env_and_falls_back_to_task_id() {
    let _guard = EnvGuard::set(&[
        (ORBIT_ACTIVE_TASK_ID_ENV, Some("ORB-00001")),
        (ORBIT_TASK_ID_ENV, Some("ORB-00002")),
    ]);
    assert_eq!(active_task_id_from_env().as_deref(), Some("ORB-00001"));
    drop(_guard);

    let _guard = EnvGuard::set(&[
        (ORBIT_ACTIVE_TASK_ID_ENV, None),
        (ORBIT_TASK_ID_ENV, Some("ORB-00002")),
    ]);
    assert_eq!(active_task_id_from_env().as_deref(), Some("ORB-00002"));
}

#[test]
fn state_file_path_matches_session_and_tmp_layouts() {
    let repo_root = Path::new("/repo");
    let tmpdir = Path::new("/tmp");
    assert_eq!(
        state_file_path(repo_root, Some("session-1"), tmpdir, 123),
        PathBuf::from("/repo/.orbit/state/sessions/session-1/review-threads.json")
    );
    assert_eq!(
        state_file_path(repo_root, None, tmpdir, 123),
        PathBuf::from("/tmp/orbit-review-thread-hook-123.json")
    );
}

#[test]
fn merge_state_tracks_last_seen_message_seq_not_thread_existence() {
    let first = reminders_from_threads(
        "ORB-00001",
        vec![thread("rt-1", 1, ReviewThreadStatus::Open)],
    );
    let (state, admitted) = merge_state(ReviewThreadHookState::new(), &first);
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0].last_seen_message_seq, 1);

    let (_state, admitted) = merge_state(state.clone(), &first);
    assert!(admitted.is_empty());

    let reply = reminders_from_threads(
        "ORB-00001",
        vec![thread("rt-1", 2, ReviewThreadStatus::Open)],
    );
    let (state, admitted) = merge_state(state, &reply);
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0].last_seen_message_seq, 2);
    assert_eq!(state.tasks["ORB-00001"]["rt-1"].last_seen_message_seq, 2);
}

#[test]
fn merge_state_advances_cursor_on_resolve_without_resurfacing() {
    let mut state = ReviewThreadHookState::new();
    state.tasks.insert(
        "ORB-00001".to_string(),
        BTreeMap::from([(
            "rt-1".to_string(),
            ReviewThreadCursor {
                last_seen_message_seq: 1,
                status: ReviewThreadStatus::Open,
            },
        )]),
    );

    let resolved = reminders_from_threads(
        "ORB-00001",
        vec![thread("rt-1", 2, ReviewThreadStatus::Resolved)],
    );
    let (state, admitted) = merge_state(state, &resolved);
    assert!(admitted.is_empty());
    let cursor = &state.tasks["ORB-00001"]["rt-1"];
    assert_eq!(cursor.last_seen_message_seq, 2);
    assert_eq!(cursor.status, ReviewThreadStatus::Resolved);
}

#[test]
fn update_state_file_persists_watermark_across_invocations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state_path = temp.path().join("state").join("review-threads.json");
    let first = reminders_from_threads(
        "ORB-00001",
        vec![thread("rt-1", 1, ReviewThreadStatus::Open)],
    );
    let admitted = update_state_file(&state_path, &first).expect("first update");
    assert_eq!(admitted.len(), 1);

    let admitted = update_state_file(&state_path, &first).expect("second update");
    assert!(admitted.is_empty());

    let persisted = std::fs::read_to_string(&state_path).expect("read state");
    let state = parse_state_json(&persisted);
    assert_eq!(state.tasks["ORB-00001"]["rt-1"].last_seen_message_seq, 1);
}

#[test]
fn render_review_thread_block_marks_task_level_and_author_identity() {
    let reminders = reminders_from_threads(
        "ORB-00001",
        vec![ReviewThread {
            thread_id: "rt-task".to_string(),
            path: None,
            line: None,
            status: ReviewThreadStatus::Open,
            messages: vec![
                ReviewMessage {
                    message_id: "rm-1".to_string(),
                    at: Utc::now(),
                    by: "human".to_string(),
                    body: "Please steer this task.".to_string(),
                    github_comment_id: None,
                },
                ReviewMessage {
                    message_id: "rm-2".to_string(),
                    at: Utc::now(),
                    by: "gpt-5.5".to_string(),
                    body: "Acknowledged.".to_string(),
                    github_comment_id: None,
                },
            ],
            github_thread_id: None,
        }],
    );

    let block = render_review_thread_block(&reminders);
    assert!(block.contains("task-level"));
    assert!(block.contains("Please steer this task."));
    assert!(block.contains("by human [human]"));
    assert!(block.contains("by gpt-5.5 [agent family codex]"));
    assert!(block.contains("Reply on the review thread with the decision"));
}

fn thread(id: &str, message_count: usize, status: ReviewThreadStatus) -> ReviewThread {
    ReviewThread {
        thread_id: id.to_string(),
        path: Some("src/lib.rs".to_string()),
        line: Some(7),
        status,
        messages: (1..=message_count)
            .map(|seq| ReviewMessage {
                message_id: format!("rm-{seq}"),
                at: Utc::now(),
                by: "human".to_string(),
                body: format!("message {seq}"),
                github_comment_id: None,
            })
            .collect(),
        github_thread_id: None,
    }
}

struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn set(values: &[(&'static str, Option<&str>)]) -> Self {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let lock = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved = values
            .iter()
            .map(|(name, _)| (*name, std::env::var(name).ok()))
            .collect::<Vec<_>>();
        for (name, value) in values {
            // SAFETY: EnvGuard serializes these process-wide mutations and restores them on drop.
            unsafe {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
        Self { _lock: lock, saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in &self.saved {
            // SAFETY: EnvGuard holds the serialization lock until all saved values are restored.
            unsafe {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}
