use orbit_common::types::{ExecutionResult, JobRunState};

use super::super::{append_runtime_model_args, map_exec_result_to_outcome};

fn execution_result(stdout: &str, success: bool) -> ExecutionResult {
    ExecutionResult {
        success,
        stdout: stdout.to_string(),
        stderr: String::new(),
        exit_code: Some(if success { 0 } else { 1 }),
        duration_ms: 12,
        output: None,
    }
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn appends_runtime_model_after_operator_args_when_flag_and_model_present() {
    let mut argv = args(&["--existing", "old"]);

    append_runtime_model_args(&mut argv, Some("--model"), Some("gpt-5.5"));

    assert_eq!(argv, args(&["--existing", "old", "--model", "gpt-5.5"]));
}

#[test]
fn leaves_args_unchanged_when_model_flag_missing() {
    let mut argv = args(&["--existing", "old"]);

    append_runtime_model_args(&mut argv, None, Some("gpt-5.5"));

    assert_eq!(argv, args(&["--existing", "old"]));
}

#[test]
fn leaves_args_unchanged_when_runtime_model_missing() {
    let mut argv = args(&["--existing", "old"]);

    append_runtime_model_args(&mut argv, Some("--model"), None);

    assert_eq!(argv, args(&["--existing", "old"]));
}

#[test]
fn leaves_args_unchanged_when_runtime_model_is_blank() {
    for model in ["", "   "] {
        let mut argv = args(&["--existing", "old"]);

        append_runtime_model_args(&mut argv, Some("--model"), Some(model));

        assert_eq!(argv, args(&["--existing", "old"]));
    }
}

#[test]
fn direct_agent_success_ignores_stdout() {
    let outcome = map_exec_result_to_outcome(&execution_result(
        r#"{"schemaVersion":1,"status":"failed","error":{"message":"ignored"}}"#,
        true,
    ));

    assert_eq!(outcome.state, JobRunState::Success);
    assert_eq!(outcome.response_json, None);
    assert_eq!(outcome.error_code, None);
}

#[test]
fn direct_agent_failure_ignores_stdout_for_error_message() {
    let outcome = map_exec_result_to_outcome(&execution_result(
        "stdout is audit data, not workflow state",
        false,
    ));

    assert_eq!(outcome.state, JobRunState::Failed);
    assert_eq!(outcome.response_json, None);
    assert_eq!(
        outcome.error_message.as_deref(),
        Some("agent execution failed with exit code Some(1)")
    );
}
