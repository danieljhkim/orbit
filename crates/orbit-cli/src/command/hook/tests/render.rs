use crate::command::hook::render::HookOutputFormat;
use chrono::Utc;
use orbit_common::types::{LearningReminder, ReviewMessage, ReviewThread, ReviewThreadStatus};
use orbit_core::command::review_thread_hook::reminders_from_threads;

#[test]
fn cli_format_converts_to_core_format() {
    assert_eq!(
        orbit_core::command::learning_hook::HookOutputFormat::from(HookOutputFormat::Claude),
        orbit_core::command::learning_hook::HookOutputFormat::Claude
    );
    assert_eq!(
        orbit_core::command::learning_hook::HookOutputFormat::from(HookOutputFormat::Codex),
        orbit_core::command::learning_hook::HookOutputFormat::Codex
    );
    assert_eq!(
        orbit_core::command::learning_hook::HookOutputFormat::from(HookOutputFormat::Gemini),
        orbit_core::command::learning_hook::HookOutputFormat::Gemini
    );
    assert_eq!(
        orbit_core::command::learning_hook::HookOutputFormat::from(HookOutputFormat::Grok),
        orbit_core::command::learning_hook::HookOutputFormat::Grok
    );
}

#[test]
fn core_renderer_combines_learning_and_review_thread_output_for_cli_formats() {
    let learnings = vec![LearningReminder {
        id: "L-0017".to_string(),
        summary: "learning reminder".to_string(),
        comments: Vec::new(),
    }];
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
                body: "async steering note".to_string(),
                github_comment_id: None,
            }],
            github_thread_id: None,
        }],
    );

    let output = orbit_core::command::learning_hook::render_hook_reminders(
        HookOutputFormat::Codex.into(),
        &learnings,
        &review_threads,
    )
    .expect("render codex hook output");
    let value: serde_json::Value = serde_json::from_str(&output).expect("parse codex JSON");
    let context = value["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additional context");
    assert!(context.contains("learning reminder"));
    assert!(context.contains("async steering note"));
    assert!(context.contains("task-level"));
}
