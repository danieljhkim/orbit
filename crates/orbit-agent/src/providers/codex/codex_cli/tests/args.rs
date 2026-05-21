#![allow(missing_docs)]

use super::super::*;

#[test]
fn codex_args_use_exec_compatible_approval_config() {
    let transport = CodexCliTransport::new(
        Some("gpt-5.5".to_string()),
        "workspace-write".to_string(),
        Some("never".to_string()),
        vec!["/tmp/orbit".to_string()],
    );

    assert_eq!(
        transport.args(),
        vec![
            "--config",
            "approval_policy=\"never\"",
            "--model",
            "gpt-5.5",
            "--sandbox",
            "workspace-write",
            "--add-dir",
            "/tmp/orbit",
        ]
    );
}
