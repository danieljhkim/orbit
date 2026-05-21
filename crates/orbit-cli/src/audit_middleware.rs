use std::time::Instant;

use orbit_common::types::{
    audit_execution_id, normalize_agent_family_for_model, normalize_optional_attribution_label,
};
use orbit_core::command::tool::take_tool_audit_recorded;
use orbit_core::{
    AuditEventInsertParams, AuditEventStatus, OrbitError, OrbitRuntime, redact_sensitive_env_text,
};
use serde_json::Value;

use crate::command::Commands;

pub struct CommandMeta {
    pub command: String,
    pub subcommand: Option<String>,
    pub tool_name: Option<String>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub role: String,
    pub arguments_json: Option<String>,
    pub job_run_id: Option<String>,
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
        Self {
            runtime,
            execution_id: audit_execution_id("exec"),
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
        // If `OrbitRuntime::execute_tool_command_dispatch` already persisted an
        // audit row for this thread (the runtime now owns tool-invocation audit
        // for both CLI and MCP entry points), suppress the guard's own emission
        // so we never double-audit a single `orbit tool run` invocation. Paths
        // that bail before the runtime is reached — invalid JSON, missing
        // input, `--dry-run` — leave the flag clear and still get a guard-side
        // row.
        if take_tool_audit_recorded() {
            return;
        }

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
            task_id: std::env::var("ORBIT_TASK_ID")
                .ok()
                .filter(|s| !s.is_empty()),
            job_run_id: self
                .meta
                .job_run_id
                .clone()
                .or_else(|| std::env::var("ORBIT_RUN_ID").ok().filter(|s| !s.is_empty())),
            activity_id: std::env::var("ORBIT_ACTIVITY_ID")
                .ok()
                .filter(|s| !s.is_empty()),
            step_index: std::env::var("ORBIT_STEP_INDEX")
                .ok()
                .and_then(|s| s.parse().ok()),
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
                job_run_id: None,
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
            job_run_id: None,
        },
        Commands::Mcp(cmd) => {
            use crate::command::mcp::McpSubcommand;
            let sub = match &cmd.command {
                McpSubcommand::Init(_) => "init",
                McpSubcommand::Remove(_) => "remove",
                McpSubcommand::Serve(_) => "serve",
            };
            CommandMeta {
                command: "mcp".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("mcp".to_string()),
                target_id: None,
                role: "admin".to_string(),
                arguments_json: None,
                job_run_id: None,
            }
        }
        Commands::Hook(cmd) => {
            use crate::command::hook::HookSubcommand;
            let sub = match &cmd.command {
                HookSubcommand::Pretooluse(_) => "pretooluse",
            };
            CommandMeta {
                command: "hook".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("hook".to_string()),
                target_id: None,
                role: "hook".to_string(),
                arguments_json: None,
                job_run_id: None,
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
            let role = match &tool_cmd.command {
                ToolSubcommand::Run(args) => tool_run_actor_role(args),
                _ => "admin".to_string(),
            };
            CommandMeta {
                command: "tool".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name,
                target_type,
                target_id,
                role,
                arguments_json: None,
                job_run_id: None,
            }
        }
        Commands::Task(task_cmd) => {
            use crate::command::task::TaskSubcommand;
            use crate::command::task::artifact::TaskArtifactSubcommand;
            let (sub, target_type, target_id) = match &task_cmd.command {
                TaskSubcommand::Add(_) => ("add", Some("task"), None),
                TaskSubcommand::Artifact(cmd) => match &cmd.command {
                    TaskArtifactSubcommand::Put(args) => {
                        ("artifact-put", Some("task"), Some(args.id.as_str()))
                    }
                },
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
                TaskSubcommand::Templates(_) => ("templates", None, None),
                TaskSubcommand::ReviewThread(_) => ("review-thread", Some("task"), None),
                TaskSubcommand::PruneContext(_) => ("prune-context", None, None),
            };
            CommandMeta {
                command: "task".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: target_type.map(String::from),
                target_id: target_id.map(String::from),
                role: "admin".to_string(),
                arguments_json: None,
                job_run_id: None,
            }
        }
        Commands::Semantic(cmd) => {
            use crate::command::semantic::SemanticSubcommand;
            let sub = match &cmd.command {
                SemanticSubcommand::Install(_) => "install",
                SemanticSubcommand::Uninstall(_) => "uninstall",
                SemanticSubcommand::Stats(_) => "stats",
                SemanticSubcommand::Index(_) => "index",
            };
            CommandMeta {
                command: "semantic".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("semantic_index".to_string()),
                target_id: None,
                role: "admin".to_string(),
                arguments_json: None,
                job_run_id: None,
            }
        }
        Commands::Search(cmd) => CommandMeta {
            command: "search".to_string(),
            // ORB-00202: preserve the `--kind` discriminator in the audit row.
            // The unified `orbit search` replaced three per-domain commands;
            // without this, downstream queries lose the ability to distinguish
            // task / doc / learning / adr searches.
            subcommand: Some(cmd.kind.to_string()),
            tool_name: None,
            target_type: Some("search".to_string()),
            target_id: None,
            role: "admin".to_string(),
            arguments_json: None,
            job_run_id: None,
        },
        Commands::Docs(cmd) => {
            use crate::command::docs::DocsSubcommand;
            let (sub, target_id) = match &cmd.command {
                DocsSubcommand::List(_) => ("list", None),
                DocsSubcommand::Show(args) => ("show", Some(args.path.as_str())),
                DocsSubcommand::Add(args) => ("add", Some(args.path.as_str())),
                DocsSubcommand::Reindex(_) => ("reindex", None),
                DocsSubcommand::Migrate(_) => ("migrate", None),
            };
            CommandMeta {
                command: "docs".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("docs".to_string()),
                target_id: target_id.map(String::from),
                role: "admin".to_string(),
                arguments_json: None,
                job_run_id: None,
            }
        }
        Commands::Learning(cmd) => {
            use crate::command::learning::LearningSubcommand;
            let sub = match &cmd.command {
                LearningSubcommand::Add(_) => "add",
                LearningSubcommand::Comment(_) => "comment",
                LearningSubcommand::List(_) => "list",
                LearningSubcommand::Show(_) => "show",
                LearningSubcommand::Update(_) => "update",
                LearningSubcommand::Upvote(_) => "upvote",
                LearningSubcommand::Supersede(_) => "supersede",
                LearningSubcommand::Reindex(_) => "reindex",
                LearningSubcommand::MigrateLayout(_) => "migrate-layout",
                LearningSubcommand::Prune(_) => "prune",
            };
            CommandMeta {
                command: "learning".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("learning".to_string()),
                target_id: None,
                role: "admin".to_string(),
                arguments_json: None,
                job_run_id: None,
            }
        }
        Commands::Activity(cmd) => {
            use crate::command::activity::ActivitySubcommand;
            let (sub, target_id): (&str, Option<&str>) = match &cmd.command {
                ActivitySubcommand::List(_) => ("list", None),
            };
            CommandMeta {
                command: "activity".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("activity".to_string()),
                target_id: target_id.map(String::from),
                role: "admin".to_string(),
                arguments_json: None,
                job_run_id: None,
            }
        }
        Commands::Job(job_cmd) => {
            use crate::command::job::JobSubcommand;
            let (sub, target_id, job_run_id) = match &job_cmd.command {
                JobSubcommand::List(_) => ("list", None, None),
                JobSubcommand::Show(args) => ("show", Some(args.job_id.as_str()), None),
                JobSubcommand::Run(args) => ("run", Some(args.job_id.as_str()), None),
                JobSubcommand::Replay(args) => (
                    "replay",
                    Some(args.run_id.as_str()),
                    Some(args.run_id.as_str()),
                ),
                JobSubcommand::RunPipelineWorker(args) => (
                    "run-pipeline-worker",
                    Some(args.run_id.as_str()),
                    Some(args.run_id.as_str()),
                ),
            };
            CommandMeta {
                command: "job".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some(if matches!(sub, "replay" | "run-pipeline-worker") {
                    "job_run".to_string()
                } else {
                    "job".to_string()
                }),
                target_id: target_id.map(String::from),
                role: "admin".to_string(),
                arguments_json: None,
                job_run_id: job_run_id.map(String::from),
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
                job_run_id: None,
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
                job_run_id: None,
            }
        }
        Commands::Graph(cmd) => {
            let sub = match &cmd.subcommand {
                crate::command::graph::GraphSubcommand::Build(_) => "build",
                crate::command::graph::GraphSubcommand::Update(_) => "update",
                crate::command::graph::GraphSubcommand::Show(_) => "show",
                crate::command::graph::GraphSubcommand::Search(_) => "search",
                crate::command::graph::GraphSubcommand::History(_) => "history",
            };
            CommandMeta {
                command: "graph".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("graph".to_string()),
                target_id: None,
                role: "admin".to_string(),
                arguments_json: None,
                job_run_id: None,
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
                job_run_id: None,
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
            job_run_id: None,
        },
        Commands::Artifacts(cmd) => CommandMeta {
            command: "artifacts".to_string(),
            subcommand: None,
            tool_name: None,
            target_type: Some(if cmd.task { "task" } else { "job_run" }.to_string()),
            target_id: Some(cmd.id.clone()),
            role: "admin".to_string(),
            arguments_json: None,
            job_run_id: None,
        },
        Commands::Web(cmd) => {
            use crate::command::web::WebSubcommand;
            let sub = match &cmd.command {
                WebSubcommand::Serve(_) => "serve",
            };
            CommandMeta {
                command: "web".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("dashboard".to_string()),
                target_id: None,
                role: "admin".to_string(),
                arguments_json: None,
                job_run_id: None,
            }
        }
        Commands::Audit(_) => unreachable!("audit commands should not be audited"),
        Commands::Log(_) => CommandMeta {
            command: "log".to_string(),
            subcommand: Some("tail".to_string()),
            tool_name: None,
            target_type: Some("log_feed".to_string()),
            target_id: None,
            role: "admin".to_string(),
            arguments_json: None,
            job_run_id: None,
        },
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
                job_run_id: None,
            }
        }
    }
}

fn run_command_meta(cmd: &crate::command::run::RunCommand) -> CommandMeta {
    use crate::command::run::RunSubcommand;

    let (subcommand, target_type, target_id) = match &cmd.command {
        RunSubcommand::Ship(_) => ("ship", Some("workflow"), Some("ship")),
        RunSubcommand::ShipLocal(_) => ("ship-local", Some("workflow"), Some("ship-local")),
        RunSubcommand::DuelPlan(args) => ("duel-plan", Some("task"), Some(args.task_id.as_str())),
        RunSubcommand::History(args) => ("history", Some("job_run"), args.job_id.as_deref()),
        RunSubcommand::Show(args) => ("show", Some("job_run"), args.run_id.as_deref()),
        RunSubcommand::Logs(args) => ("logs", Some("job_run"), args.run_id.as_deref()),
        RunSubcommand::Events(args) => ("events", Some("job_run"), args.run_id.as_deref()),
        RunSubcommand::Trace(args) => ("trace", Some("job_run"), args.run_id.as_deref()),
        RunSubcommand::Job(args) => ("job", Some("job"), Some(args.job_id.as_str())),
    };

    CommandMeta {
        command: "run".to_string(),
        subcommand: Some(subcommand.to_string()),
        tool_name: None,
        target_type: target_type.map(String::from),
        target_id: target_id.map(String::from),
        role: "admin".to_string(),
        arguments_json: None,
        job_run_id: None,
    }
}

fn tool_run_actor_role(args: &crate::command::tool::ToolRunArgs) -> String {
    let (input_agent, input_model) = tool_run_input_identity(args);
    let env_agent = std::env::var("ORBIT_AGENT_NAME")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let env_model = std::env::var("ORBIT_AGENT_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let has_input_identity = input_agent.is_some() || input_model.is_some();
    let has_flag_identity = args.agent.is_some() || args.model.is_some();
    let (agent, model) = if has_input_identity {
        (input_agent, input_model)
    } else if has_flag_identity {
        (args.agent.clone(), args.model.clone())
    } else {
        (env_agent, env_model)
    };
    let agent = normalize_agent_family_for_model(agent.as_deref(), model.as_deref())
        .ok()
        .flatten()
        .or(agent);

    normalize_optional_attribution_label(model.as_deref().or(agent.as_deref()), model.as_deref())
        .unwrap_or_else(|| "agent".to_string())
}

fn tool_run_input_identity(
    args: &crate::command::tool::ToolRunArgs,
) -> (Option<String>, Option<String>) {
    let value = args
        .input
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .or_else(|| {
            args.input_file.as_deref().and_then(|path| {
                std::fs::read_to_string(path)
                    .ok()
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            })
        });

    match value {
        Some(Value::Object(map)) => (
            map.get("agent")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            map.get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        ),
        _ => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use orbit_common::types::AuditEvent;
    use serde_json::{Value, json};
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use crate::command::Cli;

    use super::*;

    fn meta_for(args: &[&str]) -> CommandMeta {
        let cli = Cli::parse_from(args);
        extract_command_meta(&cli.command)
    }

    struct OrbitRunEnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved: Option<String>,
    }

    fn unset_orbit_run_id() -> OrbitRunEnvGuard {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let lock = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved = std::env::var("ORBIT_RUN_ID").ok();
        // SAFETY: the guard serializes this test env mutation and restores the value on drop.
        unsafe {
            std::env::remove_var("ORBIT_RUN_ID");
        }
        OrbitRunEnvGuard { _lock: lock, saved }
    }

    impl Drop for OrbitRunEnvGuard {
        fn drop(&mut self) {
            // SAFETY: the guard holds the serialization lock for the full mutation window.
            unsafe {
                match &self.saved {
                    Some(value) => std::env::set_var("ORBIT_RUN_ID", value),
                    None => std::env::remove_var("ORBIT_RUN_ID"),
                }
            }
        }
    }

    fn audit_event_for_meta_without_orbit_run_id(meta: CommandMeta) -> AuditEvent {
        let _env = unset_orbit_run_id();
        let runtime = OrbitRuntime::in_memory().expect("build in-memory runtime");
        {
            let mut guard = AuditGuard::new(&runtime, meta);
            guard.mark_success();
        }

        let events = runtime
            .list_audit_events(None, None, Some(AuditEventStatus::Success), None, 8)
            .expect("list audit events");
        assert_eq!(events.len(), 1);
        events.into_iter().next().expect("single audit event")
    }

    #[test]
    fn run_ship_audit_meta_uses_unified_workflow_alias() {
        let pr = meta_for(&["orbit", "run", "ship", "T1"]);
        assert_eq!(pr.command, "run");
        assert_eq!(pr.subcommand.as_deref(), Some("ship"));
        assert_eq!(pr.target_type.as_deref(), Some("workflow"));
        assert_eq!(pr.target_id.as_deref(), Some("ship"));

        let local = meta_for(&["orbit", "run", "ship", "-m", "local", "T1"]);
        assert_eq!(local.subcommand.as_deref(), Some("ship"));
        assert_eq!(local.target_type.as_deref(), Some("workflow"));
        assert_eq!(local.target_id.as_deref(), Some("ship"));
    }

    #[test]
    fn run_ship_local_audit_meta_uses_deprecated_top_level_command() {
        let meta = meta_for(&["orbit", "run", "ship-local", "T1"]);
        assert_eq!(meta.command, "run");
        assert_eq!(meta.subcommand.as_deref(), Some("ship-local"));
        assert_eq!(meta.target_type.as_deref(), Some("workflow"));
        assert_eq!(meta.target_id.as_deref(), Some("ship-local"));
    }

    #[test]
    fn run_duel_plan_audit_meta_targets_task() {
        let meta = meta_for(&["orbit", "run", "duel-plan", "T1"]);
        assert_eq!(meta.command, "run");
        assert_eq!(meta.subcommand.as_deref(), Some("duel-plan"));
        assert_eq!(meta.target_type.as_deref(), Some("task"));
        assert_eq!(meta.target_id.as_deref(), Some("T1"));
    }

    #[test]
    fn tool_run_audit_meta_uses_agent_flags_for_role() {
        let meta = meta_for(&[
            "orbit",
            "tool",
            "run",
            "orbit.graph.search",
            "--agent",
            "codex",
            "--model",
            "gpt-5.5",
        ]);

        assert_eq!(meta.command, "tool");
        assert_eq!(meta.subcommand.as_deref(), Some("run"));
        assert_eq!(meta.tool_name.as_deref(), Some("orbit.graph.search"));
        assert_eq!(meta.role, "gpt-5.5");
    }

    #[test]
    fn tool_run_audit_meta_uses_input_identity_for_role() {
        let meta = meta_for(&[
            "orbit",
            "tool",
            "run",
            "orbit.graph.search",
            "--input",
            r#"{"query":"actor","agent":"codex","model":"gpt-5.5"}"#,
        ]);

        assert_eq!(meta.role, "gpt-5.5");
    }

    #[test]
    fn tool_run_audit_meta_uses_model_only_input_for_role() {
        let meta = meta_for(&[
            "orbit",
            "tool",
            "run",
            "orbit.graph.search",
            "--input",
            r#"{"query":"actor","model":"gpt-5.5"}"#,
        ]);

        assert_eq!(meta.role, "gpt-5.5");
    }

    #[test]
    fn tool_run_audit_meta_prefers_input_identity_over_flags() {
        let meta = meta_for(&[
            "orbit",
            "tool",
            "run",
            "orbit.graph.search",
            "--agent",
            "codex",
            "--model",
            "gpt-5.5",
            "--input",
            r#"{"query":"actor","agent":"claude","model":"opus-4.6"}"#,
        ]);

        assert_eq!(meta.role, "opus-4.6");
    }

    #[test]
    fn tool_run_audit_meta_uses_agent_role_without_identity() {
        let meta = meta_for(&["orbit", "tool", "run", "orbit.graph.search"]);

        assert_eq!(meta.role, "agent");
    }

    #[test]
    fn search_audit_meta_preserves_kind_discriminator() {
        // ORB-00202: `orbit search --kind X` collapsed three per-domain
        // `<command> search` rows into one. Ensure the `--kind` value is
        // surfaced via `subcommand` so downstream audit queries can still
        // distinguish task / doc / learning / adr searches.
        let task = meta_for(&["orbit", "search", "foo", "--kind", "task"]);
        assert_eq!(task.command, "search");
        assert_eq!(task.subcommand.as_deref(), Some("task"));
        assert_eq!(task.target_type.as_deref(), Some("search"));

        let learning = meta_for(&["orbit", "search", "foo", "--kind", "learning"]);
        assert_eq!(learning.subcommand.as_deref(), Some("learning"));

        let doc = meta_for(&["orbit", "search", "foo", "--kind", "doc"]);
        assert_eq!(doc.subcommand.as_deref(), Some("doc"));

        let adr = meta_for(&["orbit", "search", "foo", "--kind", "adr"]);
        assert_eq!(adr.subcommand.as_deref(), Some("adr"));

        // Default `--kind all` is captured explicitly rather than left blank.
        let all = meta_for(&["orbit", "search", "foo"]);
        assert_eq!(all.subcommand.as_deref(), Some("all"));
    }

    #[test]
    fn job_run_pipeline_worker_audit_uses_static_run_id_without_env() {
        let meta = meta_for(&["orbit", "job", "run-pipeline-worker", "jrun-worker"]);
        assert_eq!(meta.command, "job");
        assert_eq!(meta.subcommand.as_deref(), Some("run-pipeline-worker"));
        assert_eq!(meta.target_type.as_deref(), Some("job_run"));
        assert_eq!(meta.target_id.as_deref(), Some("jrun-worker"));
        assert_eq!(meta.job_run_id.as_deref(), Some("jrun-worker"));

        let row = audit_event_for_meta_without_orbit_run_id(meta);
        assert_eq!(row.command, "job");
        assert_eq!(row.subcommand.as_deref(), Some("run-pipeline-worker"));
        assert_eq!(row.target_id.as_deref(), Some("jrun-worker"));
        assert_eq!(row.job_run_id.as_deref(), Some("jrun-worker"));
    }

    #[test]
    fn job_replay_audit_uses_static_run_id_without_env() {
        let meta = meta_for(&["orbit", "job", "replay", "jrun-source"]);
        assert_eq!(meta.command, "job");
        assert_eq!(meta.subcommand.as_deref(), Some("replay"));
        assert_eq!(meta.target_type.as_deref(), Some("job_run"));
        assert_eq!(meta.target_id.as_deref(), Some("jrun-source"));
        assert_eq!(meta.job_run_id.as_deref(), Some("jrun-source"));

        let row = audit_event_for_meta_without_orbit_run_id(meta);
        assert_eq!(row.command, "job");
        assert_eq!(row.subcommand.as_deref(), Some("replay"));
        assert_eq!(row.target_id.as_deref(), Some("jrun-source"));
        assert_eq!(row.job_run_id.as_deref(), Some("jrun-source"));
    }

    #[test]
    fn audit_guard_event_json_shapes_are_snapshotted() {
        let events = vec![
            audit_guard_event_json(AuditEventStatus::Success),
            audit_guard_event_json(AuditEventStatus::Failure),
            audit_guard_event_json(AuditEventStatus::Denied),
        ];

        let actual = serde_json::to_string_pretty(&events).expect("serialize audit snapshot");
        assert_eq!(
            actual,
            include_str!("snapshots/audit_guard_event_json_shapes.json").trim_end()
        );
    }

    fn audit_guard_event_json(status: AuditEventStatus) -> Value {
        let _ = orbit_core::command::tool::take_tool_audit_recorded();
        let runtime = OrbitRuntime::in_memory().expect("build in-memory runtime");
        {
            let mut guard = AuditGuard::new(&runtime, snapshot_meta());
            match status {
                AuditEventStatus::Success => guard.mark_success(),
                AuditEventStatus::Failure => {
                    let error = OrbitError::InvalidInput("snapshot failure".to_string());
                    guard.mark_failure(&error);
                }
                AuditEventStatus::Denied => guard.mark_denied("snapshot denied"),
            }
        }

        let events = runtime
            .list_audit_events(
                None,
                Some("orbit.task.update".to_string()),
                Some(status),
                None,
                8,
            )
            .expect("list audit events");
        assert_eq!(events.len(), 1);
        let mut value = serde_json::to_value(&events[0]).expect("serialize audit event");
        normalize_audit_event_json(&mut value);
        value
    }

    fn snapshot_meta() -> CommandMeta {
        CommandMeta {
            command: "tool".to_string(),
            subcommand: Some("run".to_string()),
            tool_name: Some("orbit.task.update".to_string()),
            target_type: Some("tool".to_string()),
            target_id: Some("orbit.task.update".to_string()),
            role: "gpt-5.5".to_string(),
            arguments_json: Some(r#"{"id":"ORB-00002","model":"gpt-5.5"}"#.to_string()),
            job_run_id: None,
        }
    }

    fn normalize_audit_event_json(value: &mut Value) {
        let object = value
            .as_object_mut()
            .expect("audit event serializes to object");
        object.insert("id".to_string(), json!(1));
        object.insert("execution_id".to_string(), json!("<execution_id>"));
        object.insert("timestamp".to_string(), json!("<timestamp>"));
        object.insert("duration_ms".to_string(), json!(0));
        object.insert(
            "working_directory".to_string(),
            json!("<working_directory>"),
        );
        object.insert("host".to_string(), json!("<host>"));
        object.insert("pid".to_string(), json!(0));
        object.insert("task_id".to_string(), Value::Null);
        object.insert("job_run_id".to_string(), Value::Null);
        object.insert("activity_id".to_string(), Value::Null);
        object.insert("step_index".to_string(), Value::Null);
    }

    /// Integration tests that exercise the real `AuditGuard::Drop` against an
    /// in-memory runtime, covering the four CLI `tool run` paths the
    /// dedup mechanism must handle: success-via-runtime (suppress guard
    /// emission), failure-via-runtime (suppress guard emission), invalid
    /// JSON / missing input (guard records its own row), and `--dry-run`
    /// (guard records its own row). All four must produce exactly one
    /// audit row.
    mod cli_dedup_invariant {
        use super::*;
        use orbit_core::command::tool::take_tool_audit_recorded;
        use serde_json::json;

        fn fresh_runtime() -> OrbitRuntime {
            // Reset the dedup signal so cross-test thread-local leakage
            // cannot mask a real bug in the per-call set/clear cycle.
            let _ = take_tool_audit_recorded();
            OrbitRuntime::in_memory().expect("build in-memory runtime")
        }

        fn tool_run_meta(tool_name: &str) -> CommandMeta {
            CommandMeta {
                command: "tool".to_string(),
                subcommand: Some("run".to_string()),
                tool_name: Some(tool_name.to_string()),
                target_type: Some("tool".to_string()),
                target_id: Some(tool_name.to_string()),
                role: "agent".to_string(),
                arguments_json: None,
                job_run_id: None,
            }
        }

        fn count_rows(runtime: &OrbitRuntime, tool_name: &str) -> usize {
            runtime
                .list_audit_events(None, Some(tool_name.to_string()), None, None, 16)
                .expect("list audit events")
                .len()
        }

        #[test]
        fn success_via_runtime_yields_exactly_one_row() {
            let runtime = fresh_runtime();
            {
                let mut guard = AuditGuard::new(&runtime, tool_run_meta("orbit.search"));
                let result = runtime.execute_tool_command(
                    "orbit.search",
                    json!({ "query": "anything" }),
                    None,
                    None,
                );
                assert!(result.is_ok());
                guard.mark_success();
            }
            assert_eq!(
                count_rows(&runtime, "orbit.search"),
                1,
                "runtime owns the row, guard suppressed"
            );
        }

        #[test]
        fn dispatch_failure_via_runtime_yields_exactly_one_row() {
            let runtime = fresh_runtime();
            {
                let mut guard = AuditGuard::new(&runtime, tool_run_meta("orbit.task.show"));
                let result = runtime.execute_tool_command("orbit.task.show", json!({}), None, None);
                match &result {
                    Ok(_) => panic!("expected dispatch failure"),
                    Err(err) => guard.mark_failure(err),
                }
            }
            assert_eq!(
                count_rows(&runtime, "orbit.task.show"),
                1,
                "runtime owns the row even on dispatch failure"
            );
        }

        #[test]
        fn invalid_json_bail_before_runtime_yields_exactly_one_row() {
            let runtime = fresh_runtime();
            {
                let mut guard = AuditGuard::new(&runtime, tool_run_meta("orbit.search"));
                // Simulate a CLI invalid-JSON parse failure that happens
                // before `execute_tool_command` is reached.
                let parse_err = OrbitError::InvalidInput("invalid JSON input: ...".to_string());
                guard.mark_failure(&parse_err);
                // Guard drops here without the runtime ever recording an
                // audit row.
            }
            assert_eq!(
                count_rows(&runtime, "orbit.search"),
                1,
                "guard records its own row when runtime is never reached"
            );
        }

        #[test]
        fn dry_run_bail_before_runtime_yields_exactly_one_row() {
            let runtime = fresh_runtime();
            {
                let mut guard = AuditGuard::new(&runtime, tool_run_meta("orbit.search"));
                // `--dry-run` returns Ok without invoking the runtime.
                guard.mark_success();
            }
            assert_eq!(
                count_rows(&runtime, "orbit.search"),
                1,
                "guard records its own row for the dry-run short-circuit"
            );
        }
    }
}
