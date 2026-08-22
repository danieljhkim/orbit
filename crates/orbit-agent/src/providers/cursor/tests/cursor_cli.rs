#![allow(missing_docs)]

use super::super::cursor_cli::CursorCliTransport;
use super::super::cursor_runtime::CursorRuntime;
use crate::runtime::AgentRuntime;
use crate::types::{AgentOperation, AgentRequest};

#[test]
fn args_pass_explicit_model_with_long_flag() {
    let transport = CursorCliTransport::new(Some("gpt-5".to_string()));

    assert_eq!(transport.args(), vec!["--model", "gpt-5"]);
}

#[test]
fn args_are_empty_without_a_model() {
    assert!(CursorCliTransport::new(None).args().is_empty());
}

#[test]
fn prompt_travels_on_stdin_and_never_through_argv() {
    let envelope = br#"{"schemaVersion":1,"input":{"secret_path":"/srv/tenant-42"}}"#;
    let transport = CursorCliTransport::new(Some("gpt-5".to_string()));

    let stdin = String::from_utf8(transport.stdin(envelope)).expect("utf8 stdin");
    assert!(stdin.contains("/srv/tenant-42"));
    assert!(stdin.contains("Execution envelope:"));

    let argv = transport.args().join(" ");
    assert!(!argv.contains("/srv/tenant-42"));
    assert!(!argv.contains("schemaVersion"));
}

#[test]
fn model_name_reports_the_selected_model() {
    assert_eq!(
        CursorCliTransport::new(Some("gpt-5".to_string())).model_name(),
        Some("gpt-5")
    );
    assert_eq!(CursorCliTransport::new(None).model_name(), None);
}

#[test]
fn runtime_requires_state_context_but_never_implicitly_forwards_api_key() {
    let runtime = CursorRuntime::new(
        "cursor-agent".to_string(),
        Some("gpt-5".to_string()),
        "cursor-agent",
        &["HOME", "PATH"],
    );
    let (invocation, _) = runtime
        .invoke(AgentRequest {
            operation: AgentOperation::Activity {
                activity_id: "cursor-auth-env".to_string(),
            },
            envelope_json: br#"{"schemaVersion":1,"input":{}}"#.to_vec(),
            verbose: false,
        })
        .expect("render invocation");

    assert_eq!(invocation.required_env_vars, &["HOME", "PATH"]);
    assert!(!invocation.required_env_vars.contains(&"CURSOR_API_KEY"));
    assert!(!invocation.args.iter().any(|arg| arg == "--api-key"));
}
