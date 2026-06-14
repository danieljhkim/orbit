use orbit_common::types::ToolCallTrace;
use serde_json::Value;

use super::{JsonMap, ToolCallCollector, usage::value_as_u64};

pub(super) fn extract_tool_calls(documents: &[Value]) -> Vec<ToolCallTrace> {
    let mut collector = ToolCallCollector::default();
    for document in documents {
        collector.walk(document);
    }
    collector.finish()
}

impl ToolCallCollector {
    fn walk(&mut self, value: &Value) {
        match value {
            Value::Object(map) => {
                if self.handle_wrapped_item_event(map) {
                    return;
                }

                if let Some(tool_calls) = map.get("tool_calls").and_then(Value::as_array) {
                    for item in tool_calls {
                        self.record_inline_tool_call(item);
                    }
                }

                if let Some(kind) = map.get("type").and_then(Value::as_str) {
                    match kind {
                        "tool_use" | "tool_call" | "function_call" | "custom_tool_call" => {
                            self.record_tool_use(map);
                        }
                        "tool_result"
                        | "function_call_output"
                        | "custom_tool_call_output"
                        | "command_execution" => {
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

    fn handle_wrapped_item_event(&mut self, map: &JsonMap) -> bool {
        let Some(event_kind) = map.get("type").and_then(Value::as_str) else {
            return false;
        };
        let Some(item) = map.get("item").and_then(Value::as_object) else {
            return false;
        };
        let Some(item_kind) = item.get("type").and_then(Value::as_str) else {
            return false;
        };

        match event_kind {
            "item.started" if is_tool_use_kind(item_kind) => {
                self.record_tool_use(item);
                true
            }
            "item.completed" if is_tool_use_kind(item_kind) || is_tool_result_kind(item_kind) => {
                self.record_tool_result(item);
                true
            }
            _ => false,
        }
    }

    fn record_inline_tool_call(&mut self, value: &Value) {
        let Some(map) = value.as_object() else {
            return;
        };
        let tool_name = tool_name_or_kind(map);
        if tool_name.is_empty() {
            return;
        }
        self.calls.push(ToolCallTrace {
            seq: (self.calls.len() + 1) as u32,
            tool_name: tool_name.clone(),
            result_bytes: result_bytes_from_map(map),
            result_payload: inline_result_payload(map, &tool_name),
        });
    }

    fn record_tool_use(&mut self, map: &JsonMap) {
        let tool_name = tool_name_or_kind(map);
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
        if let Some(id) = tool_call_id(map) {
            self.by_id.insert(id.to_string(), index);
        }
    }

    fn record_tool_result(&mut self, map: &JsonMap) {
        let result_bytes = result_bytes_from_map(map);

        if let Some(tool_use_id) = tool_call_id(map)
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
            if let Some(id) = tool_call_id(map) {
                self.by_id
                    .entry(id.to_string())
                    .or_insert(last.seq as usize - 1);
            }
            return;
        }

        let tool_name = tool_name_or_kind(map);
        if tool_name.is_empty() {
            return;
        }
        let index = self.calls.len();
        self.calls.push(ToolCallTrace {
            seq: (index + 1) as u32,
            tool_name: tool_name.clone(),
            result_bytes,
            result_payload: structured_result_payload(map, &tool_name),
        });
        if let Some(id) = tool_call_id(map) {
            self.by_id.insert(id.to_string(), index);
        }
    }

    fn finish(self) -> Vec<ToolCallTrace> {
        self.calls
    }
}

fn tool_name_from_map(map: &JsonMap) -> String {
    map.get("name")
        .and_then(Value::as_str)
        .or_else(|| map.get("tool_name").and_then(Value::as_str))
        .or_else(|| map.get("tool").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn tool_name_or_kind(map: &JsonMap) -> String {
    let name = tool_name_from_map(map);
    if !name.is_empty() {
        return name;
    }

    map.get("type")
        .and_then(Value::as_str)
        .filter(|kind| is_tool_use_kind(kind))
        .unwrap_or_default()
        .to_string()
}

fn tool_call_id(map: &JsonMap) -> Option<&str> {
    map.get("id")
        .and_then(Value::as_str)
        .or_else(|| map.get("tool_use_id").and_then(Value::as_str))
        .or_else(|| map.get("call_id").and_then(Value::as_str))
}

fn result_bytes_from_map(map: &JsonMap) -> u64 {
    map.get("result_bytes")
        .and_then(value_as_u64)
        .unwrap_or_else(|| result_value_from_map(map).map(serialized_size).unwrap_or(0))
}

fn inline_result_payload(map: &JsonMap, tool_name: &str) -> Option<Value> {
    if !should_capture_result_payload(tool_name) {
        return None;
    }
    result_value_from_map(map).map(normalize_captured_payload)
}

fn structured_result_payload(map: &JsonMap, tool_name: &str) -> Option<Value> {
    if !should_capture_result_payload(tool_name) {
        return None;
    }
    result_value_from_map(map).map(normalize_captured_payload)
}

fn result_value_from_map(map: &JsonMap) -> Option<&Value> {
    map.get("result")
        .or_else(|| map.get("content"))
        .or_else(|| map.get("output"))
        .or_else(|| map.get("aggregated_output"))
}

fn normalize_captured_payload(value: &Value) -> Value {
    if let Value::String(raw) = value {
        let trimmed = raw.trim();
        if (trimmed.starts_with('{') || trimmed.starts_with('['))
            && let Ok(parsed) = serde_json::from_str::<Value>(trimmed)
        {
            return parsed;
        }
    }
    value.clone()
}

fn should_capture_result_payload(tool_name: &str) -> bool {
    // ORB-00391: orbit.graph.pack was removed with orbit-knowledge (v1); only
    // fs.read payloads now feed the knowledge-stats double-read metric.
    matches!(tool_name, "fs.read")
}

fn is_tool_use_kind(kind: &str) -> bool {
    matches!(
        kind,
        "tool_use" | "tool_call" | "function_call" | "custom_tool_call" | "command_execution"
    )
}

fn is_tool_result_kind(kind: &str) -> bool {
    matches!(
        kind,
        "tool_result" | "function_call_output" | "custom_tool_call_output"
    )
}

fn serialized_size(value: &Value) -> u64 {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len() as u64)
        .unwrap_or(0)
}
