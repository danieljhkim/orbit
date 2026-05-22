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
