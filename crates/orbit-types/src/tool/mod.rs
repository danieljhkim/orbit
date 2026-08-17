//! Domain contracts for this Orbit types module.

mod definition;
mod error;
pub use error::ToolError;

pub use definition::{
    ExecutionResult, McpCapability, McpToolDefinition, McpToolDefinitionError, McpToolScope,
    McpTransport, StoredTool, ToolParam, ToolSchema, ToolSessionContext, mcp_advertised_tool_name,
    validate_mcp_tool_definitions,
};
