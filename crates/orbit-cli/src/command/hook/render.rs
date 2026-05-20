use clap::ValueEnum;
use orbit_common::types::LearningReminder;
use orbit_core::OrbitError;
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HookOutputFormat {
    Claude,
    Codex,
    Gemini,
    Grok,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
