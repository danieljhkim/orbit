use orbit_types::{
    AgentResponseEnvelope, AgentRunError, ExecutionResult, InvocationTrace, OrbitError, TokenUsage,
    ToolCallTrace,
};
use serde_json::{Deserializer, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentResponse {
    pub runtime_key: &'static str,
    pub program: String,
    pub args: Vec<String>,
    pub stdin: Vec<u8>,
    pub stdout_schema_json: Option<Value>,
    pub required_env_vars: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentResponseStatus {
    Success,
    Failed,
    Timeout,
}

pub fn parse_and_validate_response(
    exec_result: &ExecutionResult,
) -> Result<(AgentResponseEnvelope, AgentResponseStatus, InvocationTrace), OrbitError> {
    match parse_json_envelope(exec_result) {
        Ok(parsed) => Ok(parsed),
        Err(_) => Ok(synthesize_response(exec_result)),
    }
}

pub fn is_timeout(exec_result: &ExecutionResult) -> bool {
    !exec_result.success && exec_result.stderr.contains("process timed out")
}

fn parse_json_documents(stdout: &str) -> Result<Vec<Value>, OrbitError> {
    let mut documents = Vec::new();
    for item in Deserializer::from_str(stdout).into_iter::<Value>() {
        let value = item.map_err(|error| {
            OrbitError::AgentProtocolViolation(format!("stdout is not valid JSON: {error}"))
        })?;
        documents.push(value);
    }
    if documents.is_empty() {
        return Err(OrbitError::AgentProtocolViolation(
            "stdout does not contain a JSON document".to_string(),
        ));
    }
    Ok(documents)
}

fn validate_exit_alignment(
    exec_result: &ExecutionResult,
    envelope: &AgentResponseEnvelope,
) -> Result<(), OrbitError> {
    let timed_out = is_timeout(exec_result);

    if timed_out && envelope.status != "timeout" {
        return Err(OrbitError::AgentProtocolViolation(
            "timeout process must report status=timeout".to_string(),
        ));
    }

    if timed_out {
        return Ok(());
    }

    let exit_code = exec_result.exit_code.unwrap_or(1);
    if exit_code == 0 && envelope.status != "success" {
        return Err(OrbitError::AgentProtocolViolation(
            "exit_code=0 must report status=success".to_string(),
        ));
    }
    if exit_code != 0 && envelope.status == "success" {
        return Err(OrbitError::AgentProtocolViolation(
            "non-zero exit code cannot report status=success".to_string(),
        ));
    }

    Ok(())
}

fn parse_json_envelope(
    exec_result: &ExecutionResult,
) -> Result<(AgentResponseEnvelope, AgentResponseStatus, InvocationTrace), OrbitError> {
    let documents = parse_json_documents(&exec_result.stdout)?;
    let envelope = documents
        .iter()
        .rev()
        .find_map(find_agent_response_envelope)
        .ok_or_else(|| {
            OrbitError::AgentProtocolViolation(
                "stdout does not contain an Orbit response envelope".to_string(),
            )
        })?;
    let trace = extract_invocation_trace(&documents, exec_result.duration_ms);

    if envelope.schema_version != 1 {
        return Err(OrbitError::AgentProtocolViolation(format!(
            "unsupported schemaVersion: {}",
            envelope.schema_version
        )));
    }

    let state = match envelope.status.as_str() {
        "success" => AgentResponseStatus::Success,
        "failed" => {
            let Some(error) = &envelope.error else {
                return Err(OrbitError::AgentProtocolViolation(
                    "failed status requires error object".to_string(),
                ));
            };
            if error.code.trim().is_empty() {
                return Err(OrbitError::AgentProtocolViolation(
                    "failed status requires non-empty error.code".to_string(),
                ));
            }
            AgentResponseStatus::Failed
        }
        "timeout" => AgentResponseStatus::Timeout,
        other => {
            return Err(OrbitError::AgentProtocolViolation(format!(
                "unknown status: {other}"
            )));
        }
    };

    validate_exit_alignment(exec_result, &envelope)?;
    Ok((envelope, state, trace))
}

fn synthesize_response(
    exec_result: &ExecutionResult,
) -> (AgentResponseEnvelope, AgentResponseStatus, InvocationTrace) {
    if is_timeout(exec_result) {
        return (
            AgentResponseEnvelope {
                schema_version: 1,
                status: "timeout".to_string(),
                result: None,
                error: Some(AgentRunError {
                    code: "AGENT_TIMEOUT".to_string(),
                    message: "agent timed out".to_string(),
                    details: Value::Null,
                }),
                duration_ms: Some(exec_result.duration_ms),
            },
            AgentResponseStatus::Timeout,
            InvocationTrace {
                duration_ms: exec_result.duration_ms,
                ..InvocationTrace::default()
            },
        );
    }

    if exec_result.exit_code.unwrap_or(1) == 0 {
        return (
            AgentResponseEnvelope {
                schema_version: 1,
                status: "success".to_string(),
                result: None,
                error: None,
                duration_ms: Some(exec_result.duration_ms),
            },
            AgentResponseStatus::Success,
            InvocationTrace {
                duration_ms: exec_result.duration_ms,
                ..InvocationTrace::default()
            },
        );
    }

    (
        AgentResponseEnvelope {
            schema_version: 1,
            status: "failed".to_string(),
            result: None,
            error: Some(AgentRunError {
                code: "AGENT_INVOCATION_FAILED".to_string(),
                message: synthetic_error_message(exec_result),
                details: Value::Null,
            }),
            duration_ms: Some(exec_result.duration_ms),
        },
        AgentResponseStatus::Failed,
        InvocationTrace {
            duration_ms: exec_result.duration_ms,
            ..InvocationTrace::default()
        },
    )
}

fn synthetic_error_message(exec_result: &ExecutionResult) -> String {
    let stderr = exec_result.stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    let stdout = exec_result.stdout.trim();
    if !stdout.is_empty() {
        return stdout.to_string();
    }
    "agent execution failed".to_string()
}

fn find_agent_response_envelope(value: &Value) -> Option<AgentResponseEnvelope> {
    if let Some(envelope) = deserialize_envelope(value) {
        return Some(envelope);
    }

    match value {
        Value::String(raw) => {
            let nested = serde_json::from_str::<Value>(raw).ok()?;
            find_agent_response_envelope(&nested)
        }
        Value::Array(items) => items.iter().rev().find_map(find_agent_response_envelope),
        Value::Object(map) => {
            for key in [
                "result",
                "response",
                "message",
                "messages",
                "content",
                "final",
                "final_message",
                "output",
            ] {
                if let Some(found) = map.get(key).and_then(find_agent_response_envelope) {
                    return Some(found);
                }
            }

            map.values().find_map(find_agent_response_envelope)
        }
        _ => None,
    }
}

fn deserialize_envelope(value: &Value) -> Option<AgentResponseEnvelope> {
    let object = value.as_object()?;
    if !object.contains_key("schemaVersion") || !object.contains_key("status") {
        return None;
    }
    serde_json::from_value(value.clone()).ok()
}

fn extract_invocation_trace(documents: &[Value], duration_ms: u64) -> InvocationTrace {
    let usage = sum_usage(documents);
    let tool_calls = extract_tool_calls(documents);
    InvocationTrace {
        usage,
        tool_calls,
        duration_ms,
    }
}

fn sum_usage(documents: &[Value]) -> TokenUsage {
    let mut usage = TokenUsage::default();
    for document in documents {
        collect_usage(document, &mut usage, true);
    }
    usage
}

fn collect_usage(value: &Value, usage: &mut TokenUsage, allow_direct_usage: bool) {
    match value {
        Value::Object(map) => {
            if allow_direct_usage {
                if let Some(found) = usage_from_map(map) {
                    add_usage(usage, found);
                    return;
                }
            }

            if matches!(map.get("type").and_then(Value::as_str), Some("tool_result")) {
                return;
            }

            for key in ["usage", "token_usage", "tokenUsage", "tokens"] {
                if let Some(child) = map.get(key) {
                    collect_usage(child, usage, true);
                }
            }

            for (key, child) in map {
                if key != "tool_calls"
                    && key != "usage"
                    && key != "token_usage"
                    && key != "tokenUsage"
                    && key != "tokens"
                {
                    collect_usage(child, usage, false);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_usage(item, usage, allow_direct_usage);
            }
        }
        Value::String(raw) => {
            if allow_direct_usage {
                if let Ok(nested) = serde_json::from_str::<Value>(raw) {
                    collect_usage(&nested, usage, true);
                }
            }
        }
        _ => {}
    }
}

fn add_usage(usage: &mut TokenUsage, found: TokenUsage) {
    usage.input = usage.input.saturating_add(found.input);
    usage.cache_read = usage.cache_read.saturating_add(found.cache_read);
    usage.cache_create = usage.cache_create.saturating_add(found.cache_create);
    usage.output = usage.output.saturating_add(found.output);
}

fn usage_from_map(map: &serde_json::Map<String, Value>) -> Option<TokenUsage> {
    let input = first_u64(
        map,
        &[
            "input_tokens",
            "inputTokens",
            "prompt_tokens",
            "promptTokens",
        ],
    );
    let cache_read = first_u64(
        map,
        &[
            "cache_read_input_tokens",
            "cacheReadInputTokens",
            "cache_read_tokens",
            "cacheReadTokens",
        ],
    );
    let cache_create = first_u64(
        map,
        &[
            "cache_creation_input_tokens",
            "cacheCreationInputTokens",
            "cache_create_tokens",
            "cacheCreateTokens",
        ],
    );
    let output = first_u64(
        map,
        &[
            "output_tokens",
            "outputTokens",
            "completion_tokens",
            "completionTokens",
        ],
    );

    input.or(cache_read).or(cache_create).or(output)?;

    Some(TokenUsage {
        input: input.unwrap_or(0),
        cache_read: cache_read.unwrap_or(0),
        cache_create: cache_create.unwrap_or(0),
        output: output.unwrap_or(0),
    })
}

fn first_u64(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| value_as_u64(map.get(*key)?))
}

fn value_as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(raw) => raw.parse::<u64>().ok(),
        _ => None,
    }
}

fn extract_tool_calls(documents: &[Value]) -> Vec<ToolCallTrace> {
    let mut collector = ToolCallCollector::default();
    for document in documents {
        collector.walk(document);
    }
    collector.finish()
}

#[derive(Default)]
struct ToolCallCollector {
    calls: Vec<ToolCallTrace>,
    by_id: HashMap<String, usize>,
}

impl ToolCallCollector {
    fn walk(&mut self, value: &Value) {
        match value {
            Value::Object(map) => {
                if let Some(tool_calls) = map.get("tool_calls").and_then(Value::as_array) {
                    for item in tool_calls {
                        self.record_inline_tool_call(item);
                    }
                }

                if let Some(kind) = map.get("type").and_then(Value::as_str) {
                    match kind {
                        "tool_use" | "tool_call" => {
                            self.record_tool_use(map);
                        }
                        "tool_result" => {
                            self.record_tool_result(map);
                        }
                        _ => {}
                    }
                }

                for (key, nested) in map {
                    if key != "tool_calls" {
                        self.walk(nested);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.walk(item);
                }
            }
            Value::String(raw) => {
                if let Ok(nested) = serde_json::from_str::<Value>(raw) {
                    self.walk(&nested);
                }
            }
            _ => {}
        }
    }

    fn record_inline_tool_call(&mut self, value: &Value) {
        let Some(map) = value.as_object() else {
            return;
        };
        let tool_name = tool_name_from_map(map);
        if tool_name.is_empty() {
            return;
        }
        self.calls.push(ToolCallTrace {
            seq: (self.calls.len() + 1) as u32,
            tool_name: tool_name.clone(),
            result_bytes: inline_result_bytes(map),
            result_payload: inline_result_payload(map, &tool_name),
        });
    }

    fn record_tool_use(&mut self, map: &serde_json::Map<String, Value>) {
        let tool_name = tool_name_from_map(map);
        if tool_name.is_empty() {
            return;
        }
        let index = self.calls.len();
        self.calls.push(ToolCallTrace {
            seq: (index + 1) as u32,
            tool_name,
            result_bytes: 0,
            result_payload: None,
        });
        if let Some(id) = map
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| map.get("tool_use_id").and_then(Value::as_str))
        {
            self.by_id.insert(id.to_string(), index);
        }
    }

    fn record_tool_result(&mut self, map: &serde_json::Map<String, Value>) {
        let result_bytes = map
            .get("result_bytes")
            .and_then(value_as_u64)
            .unwrap_or_else(|| {
                map.get("content")
                    .or_else(|| map.get("result"))
                    .map(serialized_size)
                    .unwrap_or(0)
            });

        if let Some(tool_use_id) = map.get("tool_use_id").and_then(Value::as_str)
            && let Some(index) = self.by_id.get(tool_use_id).copied()
        {
            self.calls[index].result_bytes = result_bytes;
            self.calls[index].result_payload =
                structured_result_payload(map, &self.calls[index].tool_name);
            return;
        }

        if let Some(last) = self
            .calls
            .iter_mut()
            .rev()
            .find(|call| call.result_bytes == 0)
        {
            last.result_bytes = result_bytes;
            last.result_payload = structured_result_payload(map, &last.tool_name);
        }
    }

    fn finish(self) -> Vec<ToolCallTrace> {
        self.calls
    }
}

fn tool_name_from_map(map: &serde_json::Map<String, Value>) -> String {
    map.get("name")
        .and_then(Value::as_str)
        .or_else(|| map.get("tool_name").and_then(Value::as_str))
        .or_else(|| map.get("tool").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn inline_result_bytes(map: &serde_json::Map<String, Value>) -> u64 {
    map.get("result_bytes")
        .and_then(value_as_u64)
        .unwrap_or_else(|| {
            map.get("result")
                .or_else(|| map.get("content"))
                .map(serialized_size)
                .unwrap_or(0)
        })
}

fn inline_result_payload(map: &serde_json::Map<String, Value>, tool_name: &str) -> Option<Value> {
    if !should_capture_result_payload(tool_name) {
        return None;
    }
    map.get("result").or_else(|| map.get("content")).cloned()
}

fn structured_result_payload(
    map: &serde_json::Map<String, Value>,
    tool_name: &str,
) -> Option<Value> {
    if !should_capture_result_payload(tool_name) {
        return None;
    }
    map.get("content").or_else(|| map.get("result")).cloned()
}

fn should_capture_result_payload(tool_name: &str) -> bool {
    matches!(tool_name, "fs.read" | "orbit.knowledge.pack")
}

fn serialized_size(value: &Value) -> u64 {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{AgentResponseStatus, parse_and_validate_response};
    use orbit_types::ExecutionResult;
    use serde_json::json;

    fn exec_result(stdout: serde_json::Value) -> ExecutionResult {
        ExecutionResult {
            success: true,
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 42,
            output: None,
        }
    }

    #[test]
    fn parses_envelope_without_duration_ms() {
        let (envelope, status, trace) = parse_and_validate_response(&exec_result(json!({
            "schemaVersion": 1,
            "status": "success",
            "result": {}
        })))
        .expect("response without durationMs should parse");

        assert_eq!(status, AgentResponseStatus::Success);
        assert_eq!(envelope.duration_ms, None);
        assert_eq!(trace.duration_ms, 42);
    }

    #[test]
    fn parses_envelope_with_duration_ms_for_backwards_compatibility() {
        let (envelope, status, trace) = parse_and_validate_response(&exec_result(json!({
            "schemaVersion": 1,
            "status": "success",
            "result": {},
            "durationMs": 123
        })))
        .expect("response with durationMs should parse");

        assert_eq!(status, AgentResponseStatus::Success);
        assert_eq!(envelope.duration_ms, Some(123));
        assert_eq!(trace.duration_ms, 42);
    }

    #[test]
    fn parses_claude_json_wrapper_with_usage_and_tool_calls() {
        let stdout = [
            json!({
                "type": "assistant",
                "message": {
                    "content": [
                        { "type": "tool_use", "id": "toolu_1", "name": "fs.read" },
                        { "type": "tool_result", "tool_use_id": "toolu_1", "content": { "ok": true, "bytes": 12 } },
                        { "type": "text", "text": "{\"schemaVersion\":1,\"status\":\"success\",\"result\":{}}" }
                    ]
                }
            })
            .to_string(),
            json!({
                "type": "result",
                "usage": {
                    "input_tokens": 100,
                    "cache_read_input_tokens": 40,
                    "cache_creation_input_tokens": 5,
                    "output_tokens": 20
                }
            })
            .to_string(),
        ]
        .join("\n");
        let exec_result = ExecutionResult {
            success: true,
            stdout,
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 84,
            output: None,
        };

        let (envelope, status, trace) =
            parse_and_validate_response(&exec_result).expect("claude wrapper should parse");

        assert_eq!(status, AgentResponseStatus::Success);
        assert_eq!(envelope.status, "success");
        assert_eq!(trace.usage.input, 100);
        assert_eq!(trace.usage.cache_read, 40);
        assert_eq!(trace.usage.cache_create, 5);
        assert_eq!(trace.usage.output, 20);
        assert_eq!(trace.tool_calls.len(), 1);
        assert_eq!(trace.tool_calls[0].tool_name, "fs.read");
        assert!(trace.tool_calls[0].result_bytes > 0);
        assert_eq!(
            trace.tool_calls[0].result_payload,
            Some(json!({ "ok": true, "bytes": 12 }))
        );
    }

    #[test]
    fn sums_usage_across_multiple_llm_calls() {
        let stdout = [
            json!({
                "type": "result",
                "usage": {
                    "input_tokens": 100,
                    "cache_read_input_tokens": 10,
                    "output_tokens": 20
                }
            })
            .to_string(),
            json!({
                "type": "result",
                "usage": {
                    "input_tokens": 30,
                    "cache_creation_input_tokens": 5,
                    "output_tokens": 7
                }
            })
            .to_string(),
            json!({
                "schemaVersion": 1,
                "status": "success",
                "result": {}
            })
            .to_string(),
        ]
        .join("\n");
        let exec_result = ExecutionResult {
            success: true,
            stdout,
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 84,
            output: None,
        };

        let (_envelope, _status, trace) =
            parse_and_validate_response(&exec_result).expect("response should parse");

        assert_eq!(trace.usage.input, 130);
        assert_eq!(trace.usage.cache_read, 10);
        assert_eq!(trace.usage.cache_create, 5);
        assert_eq!(trace.usage.output, 27);
    }

    #[test]
    fn ignores_token_usage_inside_tool_result_payloads() {
        let stdout = [
            json!({
                "type": "assistant",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "toolu_1",
                            "content": {
                                "input_tokens": 999,
                                "usage": {
                                    "input_tokens": 999,
                                    "output_tokens": 999
                                }
                            }
                        }
                    ]
                }
            })
            .to_string(),
            json!({
                "type": "result",
                "usage": {
                    "input_tokens": 100,
                    "output_tokens": 20
                }
            })
            .to_string(),
            json!({
                "schemaVersion": 1,
                "status": "success",
                "result": {}
            })
            .to_string(),
        ]
        .join("\n");
        let exec_result = ExecutionResult {
            success: true,
            stdout,
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 84,
            output: None,
        };

        let (_envelope, _status, trace) =
            parse_and_validate_response(&exec_result).expect("response should parse");

        assert_eq!(trace.usage.input, 100);
        assert_eq!(trace.usage.output, 20);
    }

    #[test]
    fn captures_knowledge_pack_result_payloads() {
        let stdout = json!({
            "type": "assistant",
            "message": {
                "content": [
                    { "type": "tool_use", "id": "toolu_pack", "name": "orbit.knowledge.pack" },
                    {
                        "type": "tool_result",
                        "tool_use_id": "toolu_pack",
                        "content": {
                            "entries": [{"selector": "file:src/lib.rs"}],
                            "unresolved_selectors": ["file:src/missing.rs"]
                        }
                    },
                    { "type": "text", "text": "{\"schemaVersion\":1,\"status\":\"success\",\"result\":{}}" }
                ]
            }
        })
        .to_string();
        let exec_result = ExecutionResult {
            success: true,
            stdout,
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 12,
            output: None,
        };

        let (_, _, trace) = parse_and_validate_response(&exec_result).expect("parse");

        assert_eq!(trace.tool_calls.len(), 1);
        assert_eq!(trace.tool_calls[0].tool_name, "orbit.knowledge.pack");
        assert_eq!(
            trace.tool_calls[0].result_payload,
            Some(json!({
                "entries": [{"selector": "file:src/lib.rs"}],
                "unresolved_selectors": ["file:src/missing.rs"]
            }))
        );
    }
}
