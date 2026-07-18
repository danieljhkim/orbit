use clap::Args;
use orbit_common::types::{McpCapability, McpTransport};
use orbit_core::{AuditEventFilter, AuditEventStatus, OrbitError, OrbitRuntime};
use serde_json::Value;

use crate::command::Execute;
use crate::parse::parse_since;

use super::support::{audit_event_to_json, print_audit_event_line};

#[derive(Args)]
pub struct AuditListArgs {
    /// Filter events since duration or timestamp (e.g. "1h", "90d", RFC3339)
    #[arg(long)]
    pub since: Option<String>,
    /// Filter by tool name
    #[arg(long)]
    pub tool: Option<String>,
    /// Filter by event kind (alias for target_type)
    #[arg(long)]
    pub kind: Option<String>,
    /// Filter by status
    #[arg(long)]
    pub status: Option<AuditEventStatus>,
    /// Filter by role
    #[arg(long)]
    pub role: Option<String>,
    /// Filter by trusted logical workspace ID
    #[arg(long)]
    pub workspace: Option<String>,
    /// Filter by trusted caller machine ID
    #[arg(long)]
    pub caller_machine: Option<String>,
    /// Filter by trusted executing-process machine ID
    #[arg(long)]
    pub process_machine: Option<String>,
    /// Filter by MCP transport (`local` or `ssh-mcp`)
    #[arg(long)]
    pub transport: Option<McpTransport>,
    /// Filter by effective MCP capability membership
    #[arg(long)]
    pub capability: Option<McpCapability>,
    /// Filter by originating MCP session ID
    #[arg(long)]
    pub origin_session: Option<String>,
    /// Filter by unique MCP call ID
    #[arg(long)]
    pub mcp_call: Option<String>,
    /// Filter by canonical job run ID
    #[arg(long)]
    pub run: Option<String>,
    /// Filter by leased-run lease ID
    #[arg(long)]
    pub lease: Option<String>,
    /// Maximum number of events to return
    #[arg(long, default_value_t = 100)]
    pub limit: usize,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AuditListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let since = self.since.map(|s| parse_since(&s)).transpose()?;
        let events = runtime.list_audit_events_filtered(&AuditEventFilter {
            since,
            tool_name: self.tool,
            target_type: self.kind,
            status: self.status,
            role: self.role,
            workspace_id: self.workspace,
            caller_machine_id: self.caller_machine,
            process_machine_id: self.process_machine,
            transport: self.transport,
            capability: self.capability,
            origin_session_id: self.origin_session,
            mcp_call_id: self.mcp_call,
            job_run_id: self.run,
            lease_id: self.lease,
            limit: self.limit,
        })?;

        if self.json {
            let values: Vec<Value> = events.iter().map(audit_event_to_json).collect();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            for event in &events {
                print_audit_event_line(event);
            }
            Ok(())
        }
    }
}
