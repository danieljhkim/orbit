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
        Commands::Mcp(cmd) => {
            use crate::command::mcp::McpSubcommand;
            let sub = match &cmd.command {
                McpSubcommand::Init(_) => "init",
                McpSubcommand::Remove(_) => "remove",
            };
            CommandMeta {
                command: "mcp".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("mcp".to_string()),
                target_id: None,
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
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
                ToolSubcommand::Scaffold(args) => (
                    "scaffold",
                    args.name.clone(),
                    Some("tool".to_string()),
                    args.name.clone().or_else(|| Some(args.path.clone())),
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
                TaskSubcommand::Locks(_) => ("locks", None, None),
                TaskSubcommand::Show(args) => ("show", Some("task"), Some(args.id.as_str())),
                TaskSubcommand::Lint(args) => ("lint", Some("task"), Some(args.id.as_str())),
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
                TaskSubcommand::PruneContext(_) => ("prune-context", None, None),
                TaskSubcommand::History(_) => ("history", None, None),
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
        Commands::Activity(cmd) => {
            use crate::command::activity::ActivitySubcommand;
            let (sub, target_id): (&str, Option<&str>) = match &cmd.command {
                ActivitySubcommand::List(_) => ("list", None),
                ActivitySubcommand::Run(_) => ("run", None),
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
        Commands::Job(job_cmd) => {
            use crate::command::job::JobSubcommand;
            let (sub, target_id) = match &job_cmd.command {
                JobSubcommand::List(_) => ("list", None),
                JobSubcommand::Show(args) => ("show", Some(args.job_id.as_str())),
                JobSubcommand::Run(args) => ("run", Some(args.job_id.as_str())),
                JobSubcommand::History(args) => ("history", Some(args.job_id.as_str())),
                JobSubcommand::RunState(args) => ("run-state", Some(args.run_id.as_str())),
                JobSubcommand::RunPipelineWorker(args) => {
                    ("run-pipeline-worker", Some(args.run_id.as_str()))
                }
            };
            CommandMeta {
                command: "job".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some(if sub == "run-pipeline-worker" {
                    "job_run".to_string()
                } else {
                    "job".to_string()
                }),
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
        Commands::Executor(cmd) => {
            use crate::command::executor::ExecutorSubcommand;
            let (sub, target_id) = match &cmd.command {
                ExecutorSubcommand::List(_) => ("list", None),
                ExecutorSubcommand::Show(args) => ("show", Some(args.name.as_str())),
            };
            CommandMeta {
                command: "executor".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("executor".to_string()),
                target_id: target_id.map(String::from),
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
        Commands::Metrics(cmd) => {
            use crate::command::metrics::MetricsSubcommand;
            let (sub, target_id) = match &cmd.command {
                None => ("overview", None),
                Some(MetricsSubcommand::Overview(_)) => ("overview", None),
                Some(MetricsSubcommand::Knowledge(_)) => ("graph", None),
                Some(MetricsSubcommand::Activity(_)) => ("activity", None),
                Some(MetricsSubcommand::Task(args)) => ("task", Some(args.id.as_str())),
                Some(MetricsSubcommand::Tools(_)) => ("tools", None),
                Some(MetricsSubcommand::Invocations(_)) => ("invocations", None),
            };
            CommandMeta {
                command: "metrics".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("metrics".to_string()),
                target_id: target_id.map(String::from),
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
        Commands::Scoreboard(cmd) => {
            use crate::command::scoreboard::ScoreboardSubcommand;
            let sub = match &cmd.command {
                ScoreboardSubcommand::Summary(_) => "summary",
            };
            CommandMeta {
                command: "scoreboard".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("scoreboard".to_string()),
                target_id: None,
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
        Commands::Graph(cmd) => {
            let sub = match &cmd.subcommand {
                crate::command::graph::GraphSubcommand::Build(_) => "build",
                crate::command::graph::GraphSubcommand::Update(_) => "update",
                crate::command::graph::GraphSubcommand::Show(_) => "show",
                crate::command::graph::GraphSubcommand::Search(_) => "search",
            };
            CommandMeta {
                command: "graph".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("graph".to_string()),
                target_id: None,
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
        Commands::Policy(cmd) => {
            use crate::command::policy::PolicySubcommand;
            let (sub, target_id) = match &cmd.command {
                PolicySubcommand::List(_) => ("list", None),
                PolicySubcommand::Show(args) => ("show", Some(args.name.as_str())),
                PolicySubcommand::Check(args) => ("check", Some(args.profile_name.as_str())),
            };
            CommandMeta {
                command: "policy".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("policy".to_string()),
                target_id: target_id.map(String::from),
                role: "admin".to_string(),
                arguments_json: None,
            }
        }
        Commands::Run(cmd) => run_command_meta(cmd),
        Commands::Logs(cmd) => CommandMeta {
            command: "logs".to_string(),
            subcommand: None,
            tool_name: None,
            target_type: Some("job_run".to_string()),
            target_id: Some(cmd.run_id.clone()),
            role: "admin".to_string(),
            arguments_json: None,
        },
        Commands::Artifacts(cmd) => CommandMeta {
            command: "artifacts".to_string(),
            subcommand: None,
            tool_name: None,
            target_type: Some(if cmd.task { "task" } else { "job_run" }.to_string()),
            target_id: Some(cmd.id.clone()),
            role: "admin".to_string(),
            arguments_json: None,
        },
        Commands::Serve(cmd) => serve_command_meta(cmd),
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

fn run_command_meta(cmd: &crate::command::run::RunCommand) -> CommandMeta {
    use crate::command::run::RunSubcommand;
    use crate::command::ship::ShipMode;

    let (subcommand, target_type, target_id) = match &cmd.command {
        Some(RunSubcommand::Ship(command)) => {
            let target_id = match command.mode {
                ShipMode::Pr => "ship",
                ShipMode::Local => "ship-local",
            };
            ("ship", Some("workflow"), Some(target_id))
        }
        Some(RunSubcommand::ShipAuto(_)) => ("ship-auto", Some("workflow"), Some("ship-auto")),
        Some(RunSubcommand::DuelPlan(args)) => {
            ("duel-plan", Some("task"), Some(args.task_id.as_str()))
        }
        Some(RunSubcommand::Job(args)) => ("job", Some("job"), Some(args.job_id.as_str())),
        None => ("job", Some("job"), cmd.positional.job_id.as_deref()),
    };

    CommandMeta {
        command: "run".to_string(),
        subcommand: Some(subcommand.to_string()),
        tool_name: None,
        target_type: target_type.map(String::from),
        target_id: target_id.map(String::from),
        role: "admin".to_string(),
        arguments_json: None,
    }
}

fn serve_command_meta(cmd: &crate::command::serve::ServeCommand) -> CommandMeta {
    use crate::command::serve::ServeSubcommand;

    let (subcommand, target_type) = match &cmd.command {
        ServeSubcommand::Web(_) => ("web", "dashboard"),
        ServeSubcommand::Mcp(_) => ("mcp", "mcp"),
    };

    CommandMeta {
        command: "serve".to_string(),
        subcommand: Some(subcommand.to_string()),
        tool_name: None,
        target_type: Some(target_type.to_string()),
        target_id: None,
        role: "admin".to_string(),
        arguments_json: None,
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::command::Cli;

    use super::*;

    fn meta_for(args: &[&str]) -> CommandMeta {
        let cli = Cli::parse_from(args);
        extract_command_meta(&cli.command)
    }

    #[test]
    fn run_ship_audit_meta_uses_selected_mode_alias() {
        let pr = meta_for(&["orbit", "run", "ship", "T1"]);
        assert_eq!(pr.command, "run");
        assert_eq!(pr.subcommand.as_deref(), Some("ship"));
        assert_eq!(pr.target_type.as_deref(), Some("workflow"));
        assert_eq!(pr.target_id.as_deref(), Some("ship"));

        let local = meta_for(&["orbit", "run", "ship", "-m", "local", "T1"]);
        assert_eq!(local.subcommand.as_deref(), Some("ship"));
        assert_eq!(local.target_type.as_deref(), Some("workflow"));
        assert_eq!(local.target_id.as_deref(), Some("ship-local"));
    }

    #[test]
    fn run_ship_auto_audit_meta_uses_new_top_level_command() {
        let meta = meta_for(&["orbit", "run", "ship-auto"]);
        assert_eq!(meta.command, "run");
        assert_eq!(meta.subcommand.as_deref(), Some("ship-auto"));
        assert_eq!(meta.target_type.as_deref(), Some("workflow"));
        assert_eq!(meta.target_id.as_deref(), Some("ship-auto"));
    }

    #[test]
    fn run_duel_plan_audit_meta_targets_task() {
        let meta = meta_for(&["orbit", "run", "duel-plan", "T1"]);
        assert_eq!(meta.command, "run");
        assert_eq!(meta.subcommand.as_deref(), Some("duel-plan"));
        assert_eq!(meta.target_type.as_deref(), Some("task"));
        assert_eq!(meta.target_id.as_deref(), Some("T1"));
    }
}
