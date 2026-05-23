use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use chrono::Utc;
use fs2::FileExt;
use orbit_common::types::{
    LearningInjectionCaps, LearningReminder, ReviewMessage, ReviewThread, ReviewThreadStatus,
};

use super::super::learning_hook::{
    CODEX_PRETOOLUSE_TOOLS, HookOutputFormat, ORBIT_LEARNING_PER_CALL_CAP_ENV,
    ORBIT_LEARNING_SESSION_CAP_ENV, SessionLearningState, caps_from_env, merge_state,
    parse_payload, parse_payload_with_tools, parse_state_json, render_codex, render_gemini,
    render_reminders, state_file_path, update_state_file,
};
use super::super::review_thread_hook::reminders_from_threads;

#[test]
fn parse_payload_accepts_tool_and_path_variants() {
    let nested = parse_payload(r#"{"tool_name":"Edit","tool_input":{"file_path":" src/lib.rs "}}"#)
        .expect("nested payload");
    assert_eq!(nested.tool_name, "Edit");
    assert_eq!(nested.target_path, "src/lib.rs");

    let camel = parse_payload(r#"{"toolName":"Write","toolInput":{"filePath":"README.md"}}"#)
        .expect("camel payload");
    assert_eq!(camel.tool_name, "Write");
    assert_eq!(camel.target_path, "README.md");

    let top_level =
        parse_payload(r#"{"tool_name":"Read","path":"Cargo.toml"}"#).expect("top-level payload");
    assert_eq!(top_level.tool_name, "Read");
    assert_eq!(top_level.target_path, "Cargo.toml");
}

#[test]
fn parse_payload_rejects_malformed_irrelevant_or_pathless_payloads() {
    assert!(parse_payload("").is_none());
    assert!(parse_payload("not-json").is_none());
    assert!(parse_payload(r#"{"tool_name":"Bash","path":"src/lib.rs"}"#).is_none());
    assert!(parse_payload(r#"{"tool_name":"Edit","path":"   "}"#).is_none());
    assert!(parse_payload(r#"{"tool_name":"Edit"}"#).is_none());
}

#[test]
fn parse_payload_with_tools_accepts_codex_path_shapes() {
    let bash = parse_payload_with_tools(
        r#"{"tool_name":"Bash","tool_input":{"command":"sed -n '1,20p' crates/orbit-core/src/lib.rs"}}"#,
        CODEX_PRETOOLUSE_TOOLS,
    )
    .expect("bash payload");
    assert_eq!(bash.tool_name, "Bash");
    assert_eq!(bash.target_path, "crates/orbit-core/src/lib.rs");

    let patch = parse_payload_with_tools(
        r#"{"tool_name":"apply_patch","tool_input":{"patch":"*** Begin Patch\n*** Update File: crates/orbit-cli/src/main.rs\n@@\n*** End Patch\n"}}"#,
        CODEX_PRETOOLUSE_TOOLS,
    )
    .expect("patch payload");
    assert_eq!(patch.tool_name, "apply_patch");
    assert_eq!(patch.target_path, "crates/orbit-cli/src/main.rs");

    let mcp = parse_payload_with_tools(
        r#"{"tool_name":"mcp__plugin_orbit__fs_read","tool_input":{"filePaths":["README.md","Cargo.toml"]}}"#,
        CODEX_PRETOOLUSE_TOOLS,
    )
    .expect("mcp payload");
    assert_eq!(mcp.target_path, "README.md");
}

#[test]
fn state_file_path_matches_session_and_tmp_layouts() {
    let repo_root = Path::new("/repo");
    let tmpdir = Path::new("/tmp");
    assert_eq!(
        state_file_path(repo_root, Some("session-1"), tmpdir, 123),
        PathBuf::from("/repo/.orbit/state/sessions/session-1/learnings.json")
    );
    assert_eq!(
        state_file_path(repo_root, None, tmpdir, 123),
        PathBuf::from("/tmp/orbit-learning-hook-123.json")
    );
}

#[test]
fn parse_state_json_defaults_malformed_or_missing_count() {
    assert_eq!(parse_state_json("not-json"), SessionLearningState::new());
    let state = parse_state_json(r#"{"emitted_ids":["L2","L1"]}"#);
    assert_eq!(state.count, 2);
    assert!(state.emitted_ids.contains("L1"));
    assert!(state.emitted_ids.contains("L2"));
}

#[test]
fn merge_state_admits_cold_candidates_and_dedups_warm_state() {
    let candidates = reminders(&["L1", "L2"]);
    let caps = LearningInjectionCaps {
        per_call: 5,
        per_session_hard: 20,
    };

    let (state, admitted) = merge_state(SessionLearningState::new(), &candidates, caps);
    assert_eq!(
        admitted.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["L1", "L2"]
    );
    assert_eq!(state.count, 2);

    let (_state, admitted) = merge_state(state, &candidates, caps);
    assert!(admitted.is_empty());
}

#[test]
fn merge_state_honors_per_call_and_session_caps() {
    let candidates = reminders(&["L1", "L2", "L3", "L4"]);
    let (state, admitted) = merge_state(
        SessionLearningState::new(),
        &candidates,
        LearningInjectionCaps {
            per_call: 2,
            per_session_hard: 20,
        },
    );
    assert_eq!(
        admitted.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["L1", "L2"]
    );
    assert_eq!(state.count, 2);

    let seeded = SessionLearningState::seeded(["L1".to_string(), "L2".to_string()]);
    let (state, admitted) = merge_state(
        seeded,
        &candidates,
        LearningInjectionCaps {
            per_call: 5,
            per_session_hard: 2,
        },
    );
    assert!(admitted.is_empty());
    assert_eq!(state.count, 2);
}

#[test]
fn update_state_file_merges_watermark_and_persists_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state_path = temp.path().join("state").join("learnings.json");
    let caps = LearningInjectionCaps {
        per_call: 5,
        per_session_hard: 20,
    };

    let admitted =
        update_state_file(&state_path, &reminders(&["L1", "L2"]), caps).expect("first update");
    assert_eq!(
        admitted.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["L1", "L2"]
    );

    let admitted =
        update_state_file(&state_path, &reminders(&["L1", "L3"]), caps).expect("second update");
    assert_eq!(
        admitted.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["L3"]
    );

    let persisted = std::fs::read_to_string(&state_path).expect("read state");
    let state = parse_state_json(&persisted);
    assert_eq!(state.count, 3);
    assert!(state.emitted_ids.contains("L3"));
}

#[test]
fn update_state_file_times_out_when_lock_is_held() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state_path = temp.path().join("learnings.json");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&state_path)
        .expect("open lock file");
    file.lock_exclusive().expect("hold lock");

    let result = update_state_file(
        &state_path,
        &reminders(&["L1"]),
        LearningInjectionCaps {
            per_call: 5,
            per_session_hard: 20,
        },
    );
    file.unlock().expect("unlock");

    assert_eq!(
        result.expect_err("lock should time out"),
        "state file lock timed out"
    );
}

#[test]
fn caps_from_env_uses_shell_compatible_defaults_and_minimums() {
    let _guard = EnvGuard::set(&[
        (ORBIT_LEARNING_PER_CALL_CAP_ENV, None),
        (ORBIT_LEARNING_SESSION_CAP_ENV, None),
    ]);
    assert_eq!(
        caps_from_env(),
        LearningInjectionCaps {
            per_call: 5,
            per_session_hard: 20,
        }
    );
    drop(_guard);

    let _guard = EnvGuard::set(&[
        (ORBIT_LEARNING_PER_CALL_CAP_ENV, Some("2")),
        (ORBIT_LEARNING_SESSION_CAP_ENV, Some("0")),
    ]);
    assert_eq!(
        caps_from_env(),
        LearningInjectionCaps {
            per_call: 2,
            per_session_hard: 1,
        }
    );
    drop(_guard);

    let _guard = EnvGuard::set(&[
        (ORBIT_LEARNING_PER_CALL_CAP_ENV, Some("nope")),
        (ORBIT_LEARNING_SESSION_CAP_ENV, Some("also-nope")),
    ]);
    assert_eq!(
        caps_from_env(),
        LearningInjectionCaps {
            per_call: 5,
            per_session_hard: 20,
        }
    );
}

#[test]
fn renderers_preserve_per_agent_hook_output() {
    let reminders = reminders(&["L-0017"]);

    assert_eq!(
        render_reminders(HookOutputFormat::Grok, &reminders).expect("render grok"),
        render_reminders(HookOutputFormat::Claude, &reminders).expect("render claude"),
    );

    let codex: serde_json::Value =
        serde_json::from_str(&render_codex(&reminders).expect("render codex")).expect("parse");
    assert_eq!(
        codex["hookSpecificOutput"]["hookEventName"].as_str(),
        Some("PreToolUse")
    );
    assert!(
        codex["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additional context")
            .contains("- [L-0017] summary L-0017")
    );

    let gemini: serde_json::Value =
        serde_json::from_str(&render_gemini(&reminders).expect("render gemini")).expect("parse");
    assert_eq!(
        gemini["hookSpecificOutput"]["hookEventName"].as_str(),
        Some("BeforeTool")
    );
}

#[test]
fn render_hook_reminders_combines_learnings_and_review_threads() {
    let learnings = reminders(&["L-0017"]);
    let review_threads = reminders_from_threads(
        "ORB-00001",
        vec![ReviewThread {
            thread_id: "rt-1".to_string(),
            path: None,
            line: None,
            status: ReviewThreadStatus::Open,
            messages: vec![ReviewMessage {
                message_id: "rm-1".to_string(),
                at: Utc::now(),
                by: "human".to_string(),
                body: "Please adjust course.".to_string(),
                github_comment_id: None,
            }],
            github_thread_id: None,
        }],
    );

    let claude = super::super::learning_hook::render_hook_reminders(
        HookOutputFormat::Claude,
        &learnings,
        &review_threads,
    )
    .expect("render claude");
    assert!(claude.contains("Project learnings relevant to this task"));
    assert!(claude.contains("Review threads awaiting agent attention"));
    assert!(claude.contains("Please adjust course."));

    let grok = super::super::learning_hook::render_hook_reminders(
        HookOutputFormat::Grok,
        &learnings,
        &review_threads,
    )
    .expect("render grok");
    assert_eq!(grok, claude);

    let codex = super::super::learning_hook::render_hook_reminders(
        HookOutputFormat::Codex,
        &learnings,
        &review_threads,
    )
    .expect("render codex");
    let codex: serde_json::Value = serde_json::from_str(&codex).expect("parse codex");
    let additional_context = codex["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additional context");
    assert!(additional_context.contains("Project learnings relevant to this task"));
    assert!(additional_context.contains("Review threads awaiting agent attention"));

    let gemini = super::super::learning_hook::render_hook_reminders(
        HookOutputFormat::Gemini,
        &learnings,
        &review_threads,
    )
    .expect("render gemini");
    let gemini: serde_json::Value = serde_json::from_str(&gemini).expect("parse gemini");
    assert_eq!(
        gemini["hookSpecificOutput"]["hookEventName"].as_str(),
        Some("BeforeTool")
    );
    assert!(
        gemini["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additional context")
            .contains("Review threads awaiting agent attention")
    );
}

fn reminders(ids: &[&str]) -> Vec<LearningReminder> {
    ids.iter()
        .map(|id| LearningReminder {
            id: (*id).to_string(),
            summary: format!("summary {id}"),
            comments: Vec::new(),
        })
        .collect()
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
