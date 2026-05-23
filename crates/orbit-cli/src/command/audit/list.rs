use clap::Args;
use orbit_core::{AuditEventStatus, OrbitError, OrbitRuntime};
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
        let events = runtime.list_audit_events_with_kind(
            since,
            self.tool,
            self.kind,
            self.status,
            self.role,
            self.limit,
        )?;

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
