use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use orbit_types::telemetry::AuditEventStatus;
use orbit_types::tool::{McpCapability, McpTransport};

#[derive(Debug, Clone)]
pub struct AuditEventInsertParams {
    pub execution_id: String,
    pub command: String,
    pub subcommand: Option<String>,
    pub tool_name: Option<String>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub role: String,
    pub status: AuditEventStatus,
    pub exit_code: i32,
    pub duration_ms: i64,
    pub working_directory: String,
    pub arguments_json: Option<String>,
    pub stdout_truncated: Option<String>,
    pub stderr_truncated: Option<String>,
    pub error_message: Option<String>,
    pub host: Option<String>,
    pub pid: u32,
    pub session_id: Option<String>,
    pub workspace_id: Option<String>,
    pub caller_machine_id: Option<String>,
    pub caller_host_id: Option<String>,
    pub process_machine_id: Option<String>,
    pub process_host_id: Option<String>,
    pub transport: Option<McpTransport>,
    pub effective_capabilities: BTreeSet<McpCapability>,
    pub origin_session_id: Option<String>,
    pub mcp_call_id: Option<String>,
    pub lease_id: Option<String>,
    pub task_id: Option<String>,
    pub job_run_id: Option<String>,
    pub activity_id: Option<String>,
    pub step_index: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct AuditEventFilter {
    pub since: Option<DateTime<Utc>>,
    pub tool_name: Option<String>,
    pub target_type: Option<String>,
    pub status: Option<AuditEventStatus>,
    pub role: Option<String>,
    pub workspace_id: Option<String>,
    pub caller_machine_id: Option<String>,
    pub process_machine_id: Option<String>,
    pub transport: Option<McpTransport>,
    pub capability: Option<McpCapability>,
    pub origin_session_id: Option<String>,
    pub mcp_call_id: Option<String>,
    pub job_run_id: Option<String>,
    pub lease_id: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditToolCallCountsByRole {
    pub role: String,
    pub total: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditToolCallCountsBySurfaceAndRole {
    pub surface: String,
    pub role: String,
    pub total: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditTopToolCall {
    pub role: String,
    pub tool_name: String,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditToolAggregate {
    pub tool_name: String,
    pub total: i64,
    pub failures: i64,
    pub mcp_total: i64,
    pub cli_total: i64,
    pub mcp_failures: i64,
    pub cli_failures: i64,
    pub avg_duration_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRoleAggregate {
    pub role: String,
    pub total: i64,
    pub mcp: i64,
    pub cli: i64,
}
