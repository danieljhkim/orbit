use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::support::audit_event_to_json;

#[derive(Args)]
pub struct AuditShowArgs {
    /// Audit event ID
    pub id: i64,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AuditShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let event = runtime.show_audit_event(self.id)?;
        if self.json {
            crate::output::json::print_pretty(&audit_event_to_json(&event))
        } else {
            println!("ID:                {}", event.id);
            println!("Execution ID:      {}", event.execution_id);
            println!("Timestamp:         {}", event.timestamp.to_rfc3339());
            println!("Command:           {}", event.command);
            println!(
                "Subcommand:        {}",
                event.subcommand.as_deref().unwrap_or("-")
            );
            println!(
                "Tool:              {}",
                event.tool_name.as_deref().unwrap_or("-")
            );
            println!(
                "Target type:       {}",
                event.target_type.as_deref().unwrap_or("-")
            );
            println!(
                "Target ID:         {}",
                event.target_id.as_deref().unwrap_or("-")
            );
            println!("Role:              {}", event.role);
            println!("Status:            {}", event.status);
            println!("Exit code:         {}", event.exit_code);
            println!("Duration (ms):     {}", event.duration_ms);
            println!("Working dir:       {}", event.working_directory);
            println!("PID:               {}", event.pid);
            println!(
                "Host:              {}",
                event.host.as_deref().unwrap_or("-")
            );
            if let Some(ref err) = event.error_message {
                println!("Error:             {err}");
            }
            Ok(())
        }
    }
}
