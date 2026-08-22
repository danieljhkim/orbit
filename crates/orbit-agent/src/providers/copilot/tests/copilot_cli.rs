#![allow(missing_docs)]

use super::super::copilot_cli::CopilotCliTransport;

#[test]
fn args_pass_explicit_model_with_long_flag() {
    let transport = CopilotCliTransport::new(Some("claude-sonnet-4.5".to_string()));

    assert_eq!(transport.args(), vec!["--model", "claude-sonnet-4.5"]);
}

#[test]
fn args_propagate_a_model_from_any_vendor_verbatim() {
    // Copilot routes to several vendors. The provider identity stays
    // `copilot`; the model string is passed through untouched and is never
    // rewritten toward the vendor that supplies it. [ORB-10946]
    for model in ["gpt-5.4", "claude-opus-4.5", "gemini-3.7-flash"] {
        let transport = CopilotCliTransport::new(Some(model.to_string()));

        assert_eq!(transport.args(), vec!["--model", model]);
    }
}

#[test]
fn args_are_empty_without_a_model() {
    let transport = CopilotCliTransport::new(None);

    assert!(transport.args().is_empty());
}

#[test]
fn prompt_travels_on_stdin_and_never_through_argv() {
    // The security property behind choosing stdin over `-p <text>`: argv is
    // world-readable through process listings and is recorded in Orbit's
    // audit argv, so the execution envelope must not appear there.
    let envelope = br#"{"schemaVersion":1,"input":{"secret_path":"/srv/tenant-42"}}"#;
    let transport = CopilotCliTransport::new(Some("claude-sonnet-4.5".to_string()));

    let stdin = String::from_utf8(transport.stdin(envelope)).expect("utf8 stdin");
    assert!(stdin.contains("/srv/tenant-42"));
    assert!(stdin.contains("Execution envelope:"));

    let argv = transport.args().join(" ");
    assert!(!argv.contains("/srv/tenant-42"));
    assert!(!argv.contains("schemaVersion"));
    assert!(!argv.contains("-p"));
    assert!(!argv.contains("--prompt"));
}

#[test]
fn model_name_reports_the_selected_model() {
    assert_eq!(
        CopilotCliTransport::new(Some("gpt-5.4".to_string())).model_name(),
        Some("gpt-5.4")
    );
    assert_eq!(CopilotCliTransport::new(None).model_name(), None);
}
