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
            // ORB-00202 preserved `--kind`; ORB-00205 adds mode labels so
            // query/similar/path searches stay distinguishable in audit rows.
            subcommand: Some(cmd.audit_subcommand()),
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
                DocsSubcommand::Index(_) => ("index", None),
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
        Commands::Friction(cmd) => {
            use crate::command::friction::FrictionSubcommand;
            let (sub, target_id) = match &cmd.command {
                FrictionSubcommand::Add(_) => ("add", None),
                FrictionSubcommand::List(_) => ("list", None),
                FrictionSubcommand::Show(args) => ("show", Some(args.id.as_str())),
                FrictionSubcommand::Stats(_) => ("stats", None),
                FrictionSubcommand::Tags(_) => ("tags", None),
                FrictionSubcommand::Update(args) => ("update", Some(args.id.as_str())),
                FrictionSubcommand::Resolve(args) => ("resolve", Some(args.id.as_str())),
            };
            CommandMeta {
                command: "friction".to_string(),
                subcommand: Some(sub.to_string()),
                tool_name: None,
                target_type: Some("friction".to_string()),
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
                LearningSubcommand::Sync(_) => "sync",
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
