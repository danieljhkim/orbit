use orbit_core::AuditEvent;
use serde_json::{Value, json};

pub(super) fn audit_event_to_json(event: &AuditEvent) -> Value {
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
        "task_id": event.task_id,
        "job_run_id": event.job_run_id,
        "activity_id": event.activity_id,
        "step_index": event.step_index,
        "backend": event.backend,
    })
}

pub(super) fn print_audit_event_line(event: &AuditEvent) {
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
