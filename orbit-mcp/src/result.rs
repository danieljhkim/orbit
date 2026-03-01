use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallEnvelope {
    pub ok: bool,
    pub tool: String,
    pub args: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_block: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ToolCallError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallError {
    pub code: String,
    pub message: String,
}

impl ToolCallEnvelope {
    pub fn success(
        tool: &str,
        args: Value,
        identity_id: String,
        identity_name: String,
        identity_role: String,
        identity_block: String,
        data: Value,
    ) -> Self {
        Self {
            ok: true,
            tool: tool.to_string(),
            args,
            identity_id: Some(identity_id),
            identity_name: Some(identity_name),
            identity_role: Some(identity_role),
            identity_block: Some(identity_block),
            data: Some(data),
            error: None,
        }
    }

    pub fn failure(
        tool: &str,
        args: Value,
        identity_id: Option<String>,
        error: ToolCallError,
    ) -> Self {
        Self {
            ok: false,
            tool: tool.to_string(),
            args,
            identity_id,
            identity_name: None,
            identity_role: None,
            identity_block: None,
            data: None,
            error: Some(error),
        }
    }
}
