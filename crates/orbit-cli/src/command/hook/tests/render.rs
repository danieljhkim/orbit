use super::*;
use crate::command::hook::render::{HookOutputFormat, render_codex, render_reminders};
use orbit_common::types::LearningReminder;

fn reminders() -> Vec<LearningReminder> {
    vec![LearningReminder {
        id: "L-0017".to_string(),
        summary: "Use JSON hook context for Codex".to_string(),
        comments: Vec::new(),
    }]
}

#[test]
fn codex_renderer_wraps_reminder_block_in_json_envelope() {
    let rendered = render_codex(&reminders()).expect("render codex");
    let value: serde_json::Value = serde_json::from_str(&rendered).expect("parse JSON");
    assert_eq!(
        value["hookSpecificOutput"]["hookEventName"].as_str(),
        Some("PreToolUse")
    );
    assert!(
        value["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additional context")
            .contains("- [L-0017] Use JSON hook context for Codex")
    );
}

#[test]
fn grok_renderer_matches_claude_renderer() {
    let reminders = reminders();
    assert_eq!(
        render_reminders(HookOutputFormat::Grok, &reminders).expect("render grok"),
        render_reminders(HookOutputFormat::Claude, &reminders).expect("render claude"),
    );
}

#[test]
fn gemini_renderer_uses_before_tool_event() {
    let rendered = render_gemini(&reminders()).expect("render gemini");
    let value: serde_json::Value = serde_json::from_str(&rendered).expect("parse JSON");
    assert_eq!(
        value["hookSpecificOutput"]["hookEventName"].as_str(),
        Some("BeforeTool")
    );
}
