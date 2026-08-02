//! Typed wire layer for OpenAI-compatible chat completions.
//!
//! The transport keeps provider-specific JSON here so the higher-level module
//! can focus on translating between [`TurnRequest`] / [`TurnResponse`] and the
//! HTTP client.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct RequestMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tool_calls: Vec<OutgoingToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutgoingToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: OutgoingFunctionCall,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutgoingFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionsRequest {
    pub model: String,
    pub messages: Vec<RequestMessage>,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChatCompletionsResponse {
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub usage: IncomingUsage,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub message: IncomingMessage,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct IncomingMessage {
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default)]
    pub tool_calls: Vec<IncomingToolCall>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct IncomingToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub function: IncomingFunctionCall,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct IncomingFunctionCall {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct IncomingUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokenDetails>,
    /// Compatibility-layer aggregate when cache writes are reported beside
    /// `prompt_tokens` rather than inside `prompt_tokens_details`.
    #[serde(
        default,
        alias = "cache_creation_tokens",
        alias = "cache_write_tokens",
        alias = "cache_write_input_tokens"
    )]
    pub cache_creation_input_tokens: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PromptTokenDetails {
    #[serde(default)]
    pub cached_tokens: u64,
    /// Standard 5-minute cache writes reported by OpenAI-compatible providers.
    /// The aliases cover the field names used by common compatibility layers.
    #[serde(
        default,
        alias = "cache_creation_tokens",
        alias = "cache_creation_input_tokens",
        alias = "cache_write_input_tokens"
    )]
    pub cache_write_tokens: u64,
}
