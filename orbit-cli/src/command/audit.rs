use std::io::Write;

use clap::{Args, Subcommand, ValueEnum};
use orbit_core::{AuditEvent, AuditEventStatus, AuditStats, OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;
use crate::parse::parse_since;

#[derive(Args)]
pub struct AuditCommand {
    #[command(subcommand)]
    pub command: AuditSubcommand,
}

impl Execute for AuditCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum AuditSubcommand {
    /// List audit events
    List(AuditListArgs),
    /// Show a single audit event
    Show(AuditShowArgs),
    /// Prune old audit events
    Prune(AuditPruneArgs),
    /// Export audit events to file
    Export(AuditExportArgs),
    /// Show audit event statistics
    Stats(AuditStatsArgs),
}

impl Execute for AuditSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            AuditSubcommand::List(args) => args.execute(runtime),
            AuditSubcommand::Show(args) => args.execute(runtime),
            AuditSubcommand::Prune(args) => args.execute(runtime),
            AuditSubcommand::Export(args) => args.execute(runtime),
            AuditSubcommand::Stats(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct AuditListArgs {
    /// Filter events since duration or timestamp (e.g. "1h", "90d", RFC3339)
    #[arg(long)]
    pub since: Option<String>,
    /// Filter by tool name
    #[arg(long)]
    pub tool: Option<String>,
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
        let events =
            runtime.list_audit_events(since, self.tool, self.status, self.role, self.limit)?;

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

#[derive(Args)]
pub struct AuditPruneArgs {
    /// Prune events older than this duration (e.g. "90d", "1h")
    #[arg(long)]
    pub older_than: String,
}

impl Execute for AuditPruneArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let cutoff = parse_since(&self.older_than)?;
        let pruned = runtime.prune_audit_events(&cutoff)?;
        println!("Pruned {pruned} audit events");
        Ok(())
    }
}

#[derive(Clone, ValueEnum)]
pub enum ExportFormat {
    Json,
    Csv,
}

#[derive(Args)]
pub struct AuditExportArgs {
    /// Export format
    #[arg(long, default_value = "json")]
    pub format: ExportFormat,
    /// Output file path
    #[arg(long)]
    pub output: String,
    /// Filter events since duration or timestamp
    #[arg(long)]
    pub since: Option<String>,
    /// Filter by tool name
    #[arg(long)]
    pub tool: Option<String>,
}

impl Execute for AuditExportArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let since = self.since.map(|s| parse_since(&s)).transpose()?;
        let events = runtime.list_audit_events(since, self.tool, None, None, 0)?;

        match self.format {
            ExportFormat::Json => export_json(&self.output, &events),
            ExportFormat::Csv => export_csv(&self.output, &events),
        }
    }
}

#[derive(Args)]
pub struct AuditStatsArgs {
    /// Stats since duration or timestamp
    #[arg(long)]
    pub since: Option<String>,
    /// Filter by tool name
    #[arg(long)]
    pub tool: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AuditStatsArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let since = self.since.map(|s| parse_since(&s)).transpose()?;
        let stats = runtime.audit_event_stats(since, self.tool)?;

        if self.json {
            crate::output::json::print_pretty(&stats_to_json(&stats))
        } else {
            println!("Total:             {}", stats.total);
            println!("Success:           {}", stats.success_count);
            println!("Failure:           {}", stats.failure_count);
            println!("Denied:            {}", stats.denied_count);
            println!("Avg duration (ms): {:.1}", stats.avg_duration_ms);
            println!("P95 duration (ms): {}", stats.p95_duration_ms);
            println!("Max duration (ms): {}", stats.max_duration_ms);
            Ok(())
        }
    }
}

fn print_audit_event_line(event: &AuditEvent) {
    let tool = event.tool_name.as_deref().unwrap_or("-");
    println!(
        "[{}] {:<8} {:<6} {}:{:<20} {}ms",
        event.timestamp.format("%Y-%m-%dT%H:%M:%S"),
        event.status,
        event.role,
        event.command,
        tool,
        event.duration_ms,
    );
}

fn audit_event_to_json(event: &AuditEvent) -> Value {
    json!({
        "id": event.id,
        "execution_id": event.execution_id,
        "timestamp": event.timestamp.to_rfc3339(),
        "command": event.command,
        "subcommand": event.subcommand,
        "tool_name": event.tool_name,
        "target_type": event.target_type,
        "target_id": event.target_id,
        "role": event.role,
        "status": event.status.to_string(),
        "exit_code": event.exit_code,
        "duration_ms": event.duration_ms,
        "working_directory": event.working_directory,
        "arguments_json": event.arguments_json,
        "stdout_truncated": event.stdout_truncated,
        "stderr_truncated": event.stderr_truncated,
        "error_message": event.error_message,
        "host": event.host,
        "pid": event.pid,
        "session_id": event.session_id,
    })
}

fn stats_to_json(stats: &AuditStats) -> Value {
    json!({
        "total": stats.total,
        "success_count": stats.success_count,
        "failure_count": stats.failure_count,
        "denied_count": stats.denied_count,
        "avg_duration_ms": stats.avg_duration_ms,
        "p95_duration_ms": stats.p95_duration_ms,
        "max_duration_ms": stats.max_duration_ms,
    })
}

fn export_json(path: &str, events: &[AuditEvent]) -> Result<(), OrbitError> {
    let file =
        std::fs::File::create(path).map_err(|e| OrbitError::Io(format!("create {path}: {e}")))?;
    let mut writer = std::io::BufWriter::new(file);

    let values: Vec<Value> = events.iter().map(audit_event_to_json).collect();
    let json_bytes = serde_json::to_string_pretty(&Value::Array(values))
        .map_err(|e| OrbitError::Execution(e.to_string()))?;

    writer
        .write_all(json_bytes.as_bytes())
        .map_err(|e| OrbitError::Io(format!("write {path}: {e}")))?;
    writer
        .write_all(b"\n")
        .map_err(|e| OrbitError::Io(format!("write {path}: {e}")))?;

    println!("Exported {} events to {path}", events.len());
    Ok(())
}

fn export_csv(path: &str, events: &[AuditEvent]) -> Result<(), OrbitError> {
    let mut writer =
        csv::Writer::from_path(path).map_err(|e| OrbitError::Io(format!("create {path}: {e}")))?;

    writer
        .write_record([
            "id",
            "execution_id",
            "timestamp",
            "command",
            "subcommand",
            "tool_name",
            "target_type",
            "target_id",
            "role",
            "status",
            "exit_code",
            "duration_ms",
            "working_directory",
            "arguments_json",
            "stdout_truncated",
            "stderr_truncated",
            "error_message",
            "host",
            "pid",
            "session_id",
        ])
        .map_err(|e| OrbitError::Io(format!("write csv header: {e}")))?;

    for event in events {
        writer
            .write_record([
                event.id.to_string(),
                event.execution_id.clone(),
                event.timestamp.to_rfc3339(),
                event.command.clone(),
                event.subcommand.clone().unwrap_or_default(),
                event.tool_name.clone().unwrap_or_default(),
                event.target_type.clone().unwrap_or_default(),
                event.target_id.clone().unwrap_or_default(),
                event.role.clone(),
                event.status.to_string(),
                event.exit_code.to_string(),
                event.duration_ms.to_string(),
                event.working_directory.clone(),
                event.arguments_json.clone().unwrap_or_default(),
                event.stdout_truncated.clone().unwrap_or_default(),
                event.stderr_truncated.clone().unwrap_or_default(),
                event.error_message.clone().unwrap_or_default(),
                event.host.clone().unwrap_or_default(),
                event.pid.to_string(),
                event.session_id.clone().unwrap_or_default(),
            ])
            .map_err(|e| OrbitError::Io(format!("write csv row: {e}")))?;
    }

    writer
        .flush()
        .map_err(|e| OrbitError::Io(format!("flush csv: {e}")))?;

    println!("Exported {} events to {path}", events.len());
    Ok(())
}
