use crate::command::hook::render::HookOutputFormat;

#[test]
fn cli_format_converts_to_core_format() {
    assert_eq!(
        orbit_cmd::learning_hook::HookOutputFormat::from(HookOutputFormat::Claude),
        orbit_cmd::learning_hook::HookOutputFormat::Claude
    );
    assert_eq!(
        orbit_cmd::learning_hook::HookOutputFormat::from(HookOutputFormat::Codex),
        orbit_cmd::learning_hook::HookOutputFormat::Codex
    );
    assert_eq!(
        orbit_cmd::learning_hook::HookOutputFormat::from(HookOutputFormat::Gemini),
        orbit_cmd::learning_hook::HookOutputFormat::Gemini
    );
    assert_eq!(
        orbit_cmd::learning_hook::HookOutputFormat::from(HookOutputFormat::Grok),
        orbit_cmd::learning_hook::HookOutputFormat::Grok
    );
}
