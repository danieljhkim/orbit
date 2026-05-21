use serde_json::json;

use super::super::log_format::*;

#[test]
fn format_message_html_escapes_dynamic_field_values() {
    let html = format_message_html(
        "orbit.friction.reported",
        &json!({
            "task_id": "<script>alert(1)</script>",
            "agent": "codex",
            "model": "gpt-5.5",
            "summary": "bad <b>markup</b>"
        }),
    );

    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(html.contains("bad &lt;b&gt;markup&lt;/b&gt;"));
    assert!(!html.contains("<script>"));
}

#[test]
fn render_log_event_for_web_uses_shared_labels_and_lowercase_level() {
    let event = json!({
        "timestamp": "2026-04-27T01:00:03.000000000Z",
        "level": "WARN",
        "target": "orbit.policy.deny",
        "fields": {
            "tool": "fs.write",
            "path": "/etc/passwd",
            "profile": "writer",
            "matched_rule": "/etc/**"
        }
    });

    let rendered = render_log_event_for_web(&event);
    assert_eq!(rendered.ts, "2026-04-27T01:00:03.000000000Z");
    assert_eq!(rendered.source, "policy");
    assert_eq!(rendered.code, "DENY");
    assert_eq!(rendered.level, "warn");
    assert!(rendered.message_html.contains("<b>path</b>="));
}
