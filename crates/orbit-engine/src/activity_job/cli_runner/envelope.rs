use orbit_agent::parse_and_validate_response;
use orbit_common::types::activity_job::AgentLoopSpec;
use orbit_common::types::{ExecutionResult, InvocationTrace};
use serde_json::Value;

use super::super::dispatcher::DispatchError;

pub(super) fn cli_agent_envelope_json(
    spec: &AgentLoopSpec,
    run_id: &str,
    input: &Value,
    task_ctx: Option<&Value>,
    prompt_override: Option<&str>,
) -> Result<Vec<u8>, DispatchError> {
    let mut envelope = serde_json::Map::new();
    envelope.insert("schemaVersion".to_string(), Value::from(1));
    envelope.insert(
        "instruction".to_string(),
        Value::String(spec.instruction.clone()),
    );
    envelope.insert(
        "prompt".to_string(),
        Value::String(match prompt_override {
            Some(prompt) => prompt.to_string(),
            None => user_prompt_from_input(input)?,
        }),
    );
    envelope.insert("input".to_string(), input.clone());
    envelope.insert("run_id".to_string(), Value::String(run_id.to_string()));
    envelope.insert(
        "tools".to_string(),
        serde_json::to_value(&spec.tools)
            .map_err(|err| DispatchError::CliInvocationFailed(format!("serialize tools: {err}")))?,
    );
    envelope.insert(
        "model".to_string(),
        serde_json::to_value(&spec.model)
            .map_err(|err| DispatchError::CliInvocationFailed(format!("serialize model: {err}")))?,
    );

    if let Some(task) = task_ctx {
        envelope.insert("task".to_string(), task.clone());
    }

    serde_json::to_vec(&Value::Object(envelope))
        .map_err(|err| DispatchError::CliInvocationFailed(format!("serialize envelope: {err}")))
}

pub(super) fn parse_cli_invocation_trace(
    stdout: &[u8],
    stderr: &[u8],
    exit_code: Option<i32>,
    duration_ms: u64,
    success: bool,
) -> Option<InvocationTrace> {
    let exec_result = ExecutionResult {
        success,
        stdout: String::from_utf8_lossy(stdout).into_owned(),
        stderr: String::from_utf8_lossy(stderr).into_owned(),
        exit_code,
        duration_ms,
        output: None,
    };

    parse_and_validate_response(&exec_result)
        .map(|(_, _, trace)| trace)
        .ok()
}

pub(super) fn user_prompt_from_input(input: &Value) -> Result<String, DispatchError> {
    match input {
        Value::Object(map) => match map.get("prompt") {
            Some(Value::String(text)) => Ok(text.clone()),
            Some(other) => serde_json::to_string(other).map_err(|err| {
                DispatchError::CliInvocationFailed(format!("serialize prompt: {err}"))
            }),
            None => serde_json::to_string(input).map_err(|err| {
                DispatchError::CliInvocationFailed(format!("serialize prompt: {err}"))
            }),
        },
        Value::String(text) => Ok(text.clone()),
        Value::Null => Ok(String::new()),
        other => serde_json::to_string(other)
            .map_err(|err| DispatchError::CliInvocationFailed(format!("serialize prompt: {err}"))),
    }
}

pub(in crate::activity_job) fn task_id_from_input(input: &Value) -> Option<&str> {
    fn non_empty(value: &str) -> Option<&str> {
        if value.is_empty() { None } else { Some(value) }
    }

    input
        .get("task_id")
        .and_then(Value::as_str)
        .and_then(non_empty)
        .or_else(|| {
            input
                .get("task")
                .and_then(|task| task.get("id"))
                .and_then(Value::as_str)
                .and_then(non_empty)
        })
        .or_else(|| {
            input
                .get("task_ids")
                .and_then(Value::as_array)
                .and_then(|items| items.iter().find_map(Value::as_str))
                .and_then(non_empty)
        })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use super::super::tests::test_support::{TestHost, test_agent_loop_spec};
    use super::*;
    use orbit_common::types::TokenUsage;

    #[test]
    fn user_prompt_from_object_input_without_prompt_serializes_full_input() {
        let input = serde_json::json!({
            "failed_step_id": "push",
            "activity_name": "git_push",
            "error_message": "network timeout",
            "attempt": 2,
            "max_attempts": 2,
        });

        let prompt = user_prompt_from_input(&input).expect("prompt serializes");
        let parsed: serde_json::Value = serde_json::from_str(&prompt).expect("prompt is json");

        assert_eq!(parsed, input);
    }

    #[test]
    fn user_prompt_from_object_input_prefers_explicit_prompt() {
        let prompt = user_prompt_from_input(&serde_json::json!({
            "prompt": "do only this",
            "failed_step_id": "push",
        }))
        .expect("prompt resolves");

        assert_eq!(prompt, "do only this");
    }

    #[test]
    fn cli_agent_envelope_carries_input_run_id_and_task_context() {
        let host = TestHost {
            command: "codex".to_string(),
            executor_args: Vec::new(),
            provider_config: HashMap::new(),
            sandbox: None,
            task_context: Some(serde_json::json!({
                "id": "TCTX",
                "workspace_path": "/tmp/orbit-worktree",
                "plan": "implement it"
            })),
        };
        let spec = test_agent_loop_spec(Duration::from_secs(5));
        let input = serde_json::json!({
            "prompt": "do it",
            "task_id": "TCTX",
            "workspace_path": "/tmp/orbit-worktree"
        });

        let raw = cli_agent_envelope_json(
            &spec,
            "jrun-context",
            &input,
            host.task_context.as_ref(),
            None,
        )
        .expect("build cli agent envelope");
        let envelope: Value = serde_json::from_slice(&raw).expect("parse envelope json");

        assert_eq!(envelope["schemaVersion"], 1);
        assert_eq!(envelope["prompt"], "do it");
        assert_eq!(envelope["run_id"], "jrun-context");
        assert_eq!(envelope["input"]["task_id"], "TCTX");
        assert_eq!(envelope["input"]["workspace_path"], "/tmp/orbit-worktree");
        assert_eq!(envelope["task"]["id"], "TCTX");
        assert_eq!(envelope["task"]["workspace_path"], "/tmp/orbit-worktree");
    }

    #[test]
    fn task_id_from_input_reads_common_activity_shapes() {
        assert_eq!(
            task_id_from_input(&serde_json::json!({"task_id": "T1"})),
            Some("T1")
        );
        assert_eq!(
            task_id_from_input(&serde_json::json!({"task": {"id": "T2"}})),
            Some("T2")
        );
        assert_eq!(
            task_id_from_input(&serde_json::json!({"task_ids": ["T3", "T4"]})),
            Some("T3")
        );
        assert_eq!(task_id_from_input(&serde_json::json!({})), None);
    }

    #[test]
    fn parse_cli_invocation_trace_extracts_gemini_cli_stats_tokens() {
        let stdout = serde_json::json!({
            "result": {
                "schemaVersion": 1,
                "status": "success",
                "result": {}
            },
            "stats": {
                "models": {
                    "gemini-3.1-pro": {
                        "tokens": {
                            "input": 12,
                            "cached": 3,
                            "candidates": 4,
                            "total": 19
                        },
                        "roles": {
                            "user": {
                                "tokens": {
                                    "input": 12,
                                    "cached": 3
                                }
                            },
                            "model": {
                                "tokens": {
                                    "candidates": 4
                                }
                            }
                        }
                    }
                }
            }
        })
        .to_string();

        assert_eq!(
            parse_cli_invocation_trace(stdout.as_bytes(), b"", Some(0), 99, true)
                .map(|trace| trace.usage),
            Some(TokenUsage {
                input: 12,
                cache_read: 3,
                cache_create: 0,
                output: 4,
            })
        );
    }

    #[test]
    fn parse_cli_invocation_trace_accepts_grok_json_text_envelope() {
        let stdout = serde_json::json!({
            "text": "{\"schemaVersion\":1,\"status\":\"success\",\"result\":{\"pong\":\"grok\"},\"error\":null}",
            "stopReason": "EndTurn",
            "sessionId": "grok-session",
            "requestId": "grok-request"
        })
        .to_string();

        assert!(
            parse_cli_invocation_trace(stdout.as_bytes(), b"", Some(0), 99, true).is_some(),
            "grok --output-format json stdout should expose the embedded Orbit envelope"
        );
    }
}
