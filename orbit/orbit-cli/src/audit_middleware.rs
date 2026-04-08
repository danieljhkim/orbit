use std::time::Instant;

use orbit_core::{
    AuditEventInsertParams, AuditEventStatus, OrbitError, OrbitRuntime, redact_sensitive_env_text,
};

use crate::command::Commands;

pub struct CommandMeta {
    pub command: String,
    pub subcommand: Option<String>,
    pub tool_name: Option<String>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub role: String,
    pub arguments_json: Option<String>,
}

/// Feeds the **persistent** SQLite audit event store on every CLI invocation.
///
/// This is a separate mechanism from the in-process `EventLog` (`OrbitRuntime.event_log`), which
/// is session-scoped and not persisted. The two channels serve different purposes:
/// - `AuditGuard` (this file): records structured CLI invocation metadata to SQLite; survives
///   process restarts; queryable via `orbit audit list`.
/// - `EventLog`: records in-memory `OrbitEvent` mutations for the current session only; used for
///   internal runtime tracking, not for persistent audit history.
///
/// RAII audit guard that writes an audit record on scope exit via `Drop`.
///
/// Guarantees exactly one audit record per command execution — even on
/// early returns or panics (with `panic = "unwind"`).
///
/// Status defaults to `Failure` with exit code -1 if never explicitly marked.
pub struct AuditGuard<'a> {
    runtime: &'a OrbitRuntime,
    execution_id: String,
    meta: CommandMeta,
    start: Instant,
    status: AuditEventStatus,
    exit_code: i32,
    error_message: Option<String>,
}

impl<'a> AuditGuard<'a> {
    pub fn new(runtime: &'a OrbitRuntime, meta: CommandMeta) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);

        Self {
            runtime,
            execution_id: format!("exec-{nanos}"),
            meta,
            start: Instant::now(),
            status: AuditEventStatus::Failure,
            exit_code: -1,
            error_message: None,
        }
    }

    pub fn mark_success(&mut self) {
        self.status = AuditEventStatus::Success;
        self.exit_code = 0;
        self.error_message = None;
    }

    pub fn mark_failure(&mut self, error: &OrbitError) {
        self.status = AuditEventStatus::Failure;
        self.exit_code = 1;
        self.error_message = Some(redact_sensitive_env_text(&error.to_string()));
    }

    pub fn mark_denied(&mut self, msg: &str) {
        self.status = AuditEventStatus::Denied;
        self.exit_code = 1;
        self.error_message = Some(redact_sensitive_env_text(msg));
    }
}

impl Drop for AuditGuard<'_> {
    fn drop(&mut self) {
        let duration_ms = self.start.elapsed().as_millis() as i64;

        let working_directory = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());

        let params = AuditEventInsertParams {
            execution_id: self.execution_id.clone(),
            command: self.meta.command.clone(),
            subcommand: self.meta.subcommand.clone(),
            tool_name: self.meta.tool_name.clone(),
            target_type: self.meta.target_type.clone(),
            target_id: self.meta.target_id.clone(),
            role: self.meta.role.clone(),
            status: self.status,
            exit_code: self.exit_code,
            duration_ms,
            working_directory,
            arguments_json: self.meta.arguments_json.clone(),
            stdout_truncated: None,
            stderr_truncated: None,
            error_message: self.error_message.clone(),
            host: std::env::var("HOSTNAME").ok(),
            pid: std::process::id(),
            session_id: None,
        };

        let write_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.record_audit_event(&params)
        }));

        match write_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                eprintln!("warning: failed to write audit event: {e}");
            }
            Err(_) => {
                eprintln!("critical: audit panic during drop");
            }
        }
    }
}

pub fn extract_command_meta(cmd: &Commands) -> CommandMeta {
    match cmd {
        Commands::Config(config_cmd) => {
            use crate::command::config::ConfigSubcommand;
            let sub = match &config_cmd.command {
                ConfigSubcommand::Show(_) => "show",
            };
            CommandMeta {
                command: "config".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("config".to_string()),
                target_id: None,
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
        Commands::Init(_) => CommandMeta {
            command: "init".to_string(),
            subcommand: None,
            tool_name: None,
            target_type: Some("config".to_string()),
            target_id: None,
            role: "admin".to_string(),
            arguments_json: None,
        },
        Commands::Tool(tool_cmd) => {
            use crate::command::tool::ToolSubcommand;
            let (sub, tool_name, target_type, target_id) = match &tool_cmd.command {
                ToolSubcommand::Run(args) => (
                    "run",
                    Some(args.name.clone()),
                    Some("tool".to_string()),
                    Some(args.name.clone()),
                ),
                ToolSubcommand::List(_) => ("list", None, None, None),
                ToolSubcommand::Show(args) => (
                    "show",
                    Some(args.name.clone()),
                    Some("tool".to_string()),
                    Some(args.name.clone()),
                ),
                ToolSubcommand::Add(args) => (
                    "add",
                    args.name.clone(),
                    Some("tool".to_string()),
                    args.name.clone(),
                ),
                ToolSubcommand::Remove(args) => (
                    "remove",
                    Some(args.name.clone()),
                    Some("tool".to_string()),
                    Some(args.name.clone()),
                ),
                ToolSubcommand::Enable(args) => (
                    "enable",
                    Some(args.name.clone()),
                    Some("tool".to_string()),
                    Some(args.name.clone()),
                ),
                ToolSubcommand::Disable(args) => (
                    "disable",
                    Some(args.name.clone()),
                    Some("tool".to_string()),
                    Some(args.name.clone()),
                ),
                ToolSubcommand::Doctor => ("doctor", None, None, None),
            };
            CommandMeta {
                command: "tool".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name,
                target_type,
                target_id,
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
        Commands::Task(task_cmd) => {
            use crate::command::task::TaskSubcommand;
            let (sub, target_type, target_id) = match &task_cmd.command {
                TaskSubcommand::Add(_) => ("add", Some("task"), None),
                TaskSubcommand::List(_) => ("list", None, None),
                TaskSubcommand::Show(args) => ("show", Some("task"), Some(args.id.as_str())),
                TaskSubcommand::Update(args) => ("update", Some("task"), Some(args.id.as_str())),
                TaskSubcommand::Start(args) => ("start", Some("task"), Some(args.id.as_str())),
                TaskSubcommand::Approve(args) => (
                    "approve",
                    Some("task"),
                    args.ids.first().map(|s| s.as_str()),
                ),
                TaskSubcommand::Reject(args) => {
                    ("reject", Some("task"), args.ids.first().map(|s| s.as_str()))
                }
                TaskSubcommand::Archive(args) => ("archive", Some("task"), Some(args.id.as_str())),
                TaskSubcommand::Unarchive(args) => {
                    ("unarchive", Some("task"), Some(args.id.as_str()))
                }
                TaskSubcommand::Delete(args) => ("delete", Some("task"), Some(args.id.as_str())),
                TaskSubcommand::Search(_) => ("search", None, None),
                TaskSubcommand::Templates(_) => ("templates", None, None),
                TaskSubcommand::ReviewThread(_) => ("review-thread", Some("task"), None),
            };
            CommandMeta {
                command: "task".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: target_type.map(String::from),
                target_id: target_id.map(String::from),
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
        Commands::Job(job_cmd) => {
            use crate::command::job::JobSubcommand;
            let (sub, target_id) = match &job_cmd.command {
                JobSubcommand::Add(_) => ("add", None),
                JobSubcommand::List(_) => ("list", None),
                JobSubcommand::Show(args) => ("show", Some(args.job_id.as_str())),
                JobSubcommand::Run(args) => ("run", Some(args.job_id.as_str())),
                JobSubcommand::History(args) => ("history", Some(args.job_id.as_str())),
                JobSubcommand::Delete(args) => ("delete", Some(args.job_id.as_str())),
            };
            CommandMeta {
                command: "job".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("job".to_string()),
                target_id: target_id.map(String::from),
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
        Commands::JobRun(job_run_cmd) => {
            use crate::command::job_run::JobRunSubcommand;
            let (sub, target_id) = match &job_run_cmd.command {
                JobRunSubcommand::List(_) => ("list", None),
                JobRunSubcommand::Show(args) => ("show", Some(args.run_id.as_str())),
                JobRunSubcommand::Cancel(args) => ("cancel", Some(args.run_id.as_str())),
                JobRunSubcommand::Archive(args) => ("archive", Some(args.run_id.as_str())),
                JobRunSubcommand::Delete(args) => ("delete", Some(args.run_id.as_str())),
                JobRunSubcommand::Retry(args) => ("retry", Some(args.run_id.as_str())),
            };
            CommandMeta {
                command: "job-run".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("job_run".to_string()),
                target_id: target_id.map(String::from),
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
        Commands::Activity(cmd) => {
            use crate::command::activity::ActivitySubcommand;
            let (sub, target_id) = match &cmd.command {
                ActivitySubcommand::Add(args) => ("add", Some(args.id.as_str())),
                ActivitySubcommand::List(_) => ("list", None),
                ActivitySubcommand::Show(args) => ("show", Some(args.id.as_str())),
                ActivitySubcommand::Update(args) => ("update", Some(args.id.as_str())),
                ActivitySubcommand::Run(args) => ("run", Some(args.id.as_str())),
                ActivitySubcommand::Delete(args) => ("delete", Some(args.id.as_str())),
            };
            CommandMeta {
                command: "activity".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("activity".to_string()),
                target_id: target_id.map(String::from),
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
        Commands::Skill(cmd) => {
            use crate::command::skill::SkillSubcommand;
            let (sub, target_id) = match &cmd.command {
                SkillSubcommand::List(_) => ("list", None),
                SkillSubcommand::Show(args) => ("show", Some(args.name.as_str())),
                SkillSubcommand::Doctor(_) => ("doctor", None),
                SkillSubcommand::Link(_) => ("link", None),
                SkillSubcommand::Unlink(_) => ("unlink", None),
            };
            CommandMeta {
                command: "skill".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("skill".to_string()),
                target_id: target_id.map(String::from),
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
        Commands::Run(cmd) => {
            let target_id = cmd.workflow.as_deref();
            CommandMeta {
                command: "run".to_string(),
                subcommand: target_id.map(String::from),
                tool_name: None,
                target_type: Some("workflow".to_string()),
                target_id: target_id.map(String::from),
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
        Commands::Audit(_) => unreachable!("audit commands should not be audited"),
        Commands::Workspace(cmd) => {
            use crate::command::workspace::WorkspaceSubcommand;
            let sub = match &cmd.command {
                WorkspaceSubcommand::Init(_) => "init",
                WorkspaceSubcommand::List(_) => "list",
                WorkspaceSubcommand::Show(_) => "show",
                WorkspaceSubcommand::Remove(_) => "remove",
                WorkspaceSubcommand::Teardown(_) => "teardown",
            };
            CommandMeta {
                command: "workspace".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("workspace".to_string()),
                target_id: None,
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
    }
}
