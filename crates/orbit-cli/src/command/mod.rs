pub mod definitions;
pub mod docs;
pub mod environment;
pub mod hook;
pub mod learning;
pub mod log;
pub mod mcp;
pub mod observe;
pub mod run;
pub mod search;
pub mod semantic;
pub mod task;
pub mod web;

pub use definitions::{activity, executor, job, policy, skill, tool};
pub use environment::{config, init, workspace};
pub use observe::{audit, graph};

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

pub trait Execute {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError>;
}

// Clap derive does not support per-variant subcommand `help_heading`
// (`next_help_heading` is args-only; `subcommand_help_heading` only renames
// the single `Commands:` block). To render grouped sections in `--help` we
// hand-roll the template below. Keep the variant order and the template's
// section order in sync when adding new commands — the variant order also
// determines where a missing-from-template command would otherwise appear.
#[derive(Parser)]
#[command(name = "orbit")]
#[command(about = "Orbit CLI", version)]
#[command(
    disable_help_subcommand = true,
    help_template = "\
{name} {version}

{usage-heading} {usage}

Environment:
  init        Initialize the global Orbit root (~/.orbit)
  workspace   Manage workspaces
  config      Show or update Orbit configuration
  semantic    Manage local orbit-search indexing

Operate:
  run         Run a workflow (ship, duel-plan, job)
  task        Create, update, and manage tasks
  search      Search tasks, docs, learnings, and ADRs
  docs        Search and manage the indexed docs corpus
  learning    Create, search, and curate project learnings

Observe:
  graph       Query the knowledge graph
  audit       Query the audit event log
  log         Tail the unified Orbit log feed

Definitions:
  activity    View activity definitions
  job         View job definitions
  tool        View tool registry
  policy      View filesystem policies
  executor    View executors

Services:
  mcp         Register MCP client integrations and run the MCP server
  hook        Run Orbit-owned editor hooks
  web         Run the Orbit dashboard

Options:
{options}"
)]
pub struct Cli {
    /// Override the Orbit root directory (highest precedence)
    #[arg(long, global = true)]
    pub root: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    // ── Environment ──
    Init(init::InitCommand),
    Workspace(workspace::WorkspaceCommand),
    Config(config::ConfigCommand),
    Semantic(semantic::SemanticCommand),

    // ── Operate ──
    Run(run::RunCommand),
    Task(Box<task::TaskCommand>),
    Search(search::SearchCommand),
    Docs(docs::DocsCommand),
    Learning(learning::LearningCommand),

    // ── Observe ──
    Graph(graph::GraphCommand),
    Audit(audit::AuditCommand),
    Log(log::LogCommand),

    // ── Definitions ──
    Activity(activity::ActivityCommand),
    Job(job::JobCommand),
    Tool(tool::ToolCommand),
    Policy(policy::PolicyCommand),
    Executor(executor::ExecutorCommand),

    // ── Services ──
    Mcp(mcp::McpCommand),
    Hook(hook::HookCommand),
    Web(web::WebCommand),

    // ── hidden compatibility commands ──
    #[command(hide = true)]
    Skill(skill::SkillCommand),
    #[command(hide = true)]
    Logs(run::legacy_logs::LogsCommand),
    #[command(hide = true)]
    Artifacts(task::artifacts::ArtifactsCommand),
}

impl Execute for Commands {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            Commands::Init(cmd) => cmd.execute(runtime),
            Commands::Workspace(cmd) => cmd.execute(runtime),
            Commands::Config(cmd) => cmd.execute(runtime),
            Commands::Semantic(cmd) => cmd.execute(runtime),
            Commands::Run(cmd) => cmd.execute(runtime),
            Commands::Task(cmd) => (*cmd).execute(runtime),
            Commands::Search(cmd) => cmd.execute(runtime),
            Commands::Docs(cmd) => cmd.execute(runtime),
            Commands::Learning(cmd) => cmd.execute(runtime),
            Commands::Graph(cmd) => cmd.execute(runtime),
            Commands::Audit(cmd) => cmd.execute(runtime),
            Commands::Log(cmd) => cmd.execute(runtime),
            Commands::Activity(cmd) => cmd.execute(runtime),
            Commands::Job(cmd) => cmd.execute(runtime),
            Commands::Tool(cmd) => cmd.execute(runtime),
            Commands::Policy(cmd) => cmd.execute(runtime),
            Commands::Executor(cmd) => cmd.execute(runtime),
            Commands::Mcp(cmd) => cmd.execute(runtime),
            Commands::Hook(cmd) => cmd.execute(runtime),
            Commands::Web(cmd) => cmd.execute(runtime),
            Commands::Skill(cmd) => cmd.execute(runtime),
            Commands::Logs(cmd) => cmd.execute(runtime),
            Commands::Artifacts(cmd) => cmd.execute(runtime),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{
        Cli, Commands,
        hook::HookSubcommand,
        mcp::McpSubcommand,
        search::{SearchKindArg, SearchSubcommand},
        semantic::SemanticSubcommand,
        web::WebSubcommand,
    };

    #[test]
    fn cli_parses_mcp_init() {
        let cli = Cli::parse_from(["orbit", "mcp", "init"]);
        match cli.command {
            Commands::Mcp(command) => match command.command {
                McpSubcommand::Init(_) => {}
                _ => panic!("expected mcp init"),
            },
            _ => panic!("expected top-level mcp command"),
        }
    }

    #[test]
    fn cli_parses_mcp_serve() {
        let cli = Cli::parse_from(["orbit", "mcp", "serve"]);
        match cli.command {
            Commands::Mcp(command) => match command.command {
                McpSubcommand::Serve(_) => {}
                _ => panic!("expected mcp serve"),
            },
            _ => panic!("expected top-level mcp command"),
        }
    }

    #[test]
    fn cli_parses_web_serve() {
        let cli = Cli::parse_from(["orbit", "web", "serve"]);
        match cli.command {
            Commands::Web(command) => match command.command {
                WebSubcommand::Serve(_) => {}
            },
            _ => panic!("expected top-level web command"),
        }
    }

    #[test]
    fn cli_parses_hook_pretooluse() {
        let cli = Cli::parse_from(["orbit", "hook", "pretooluse", "--format", "codex"]);
        match cli.command {
            Commands::Hook(command) => match command.command {
                HookSubcommand::Pretooluse(_) => {}
            },
            _ => panic!("expected top-level hook command"),
        }
    }

    #[test]
    fn cli_parses_semantic_install_force() {
        let cli = Cli::parse_from(["orbit", "semantic", "install", "--force"]);
        match cli.command {
            Commands::Semantic(command) => match command.command {
                SemanticSubcommand::Install(args) => assert!(args.force),
                _ => panic!("expected semantic install"),
            },
            _ => panic!("expected top-level semantic command"),
        }
    }

    #[test]
    fn cli_parses_semantic_stats() {
        let cli = Cli::parse_from(["orbit", "semantic", "stats"]);
        match cli.command {
            Commands::Semantic(command) => match command.command {
                SemanticSubcommand::Stats(_) => {}
                _ => panic!("expected semantic stats"),
            },
            _ => panic!("expected top-level semantic command"),
        }
    }

    #[test]
    fn cli_parses_semantic_index() {
        let cli = Cli::parse_from(["orbit", "semantic", "index", "--force"]);
        match cli.command {
            Commands::Semantic(command) => match command.command {
                SemanticSubcommand::Index(args) => assert!(args.force),
                _ => panic!("expected semantic index"),
            },
            _ => panic!("expected top-level semantic command"),
        }
    }

    #[test]
    fn cli_parses_top_level_search() {
        let cli = Cli::parse_from([
            "orbit",
            "search",
            "semantic search design",
            "--hybrid",
            "--kind",
            "task",
        ]);
        match cli.command {
            Commands::Search(args) => {
                assert_eq!(args.query.as_deref(), Some("semantic search design"));
                assert!(args.hybrid);
                assert!(args.command.is_none());
            }
            _ => panic!("expected top-level search command"),
        }
    }

    #[test]
    fn cli_parses_top_level_search_similar_neighbor() {
        let cli = Cli::parse_from(["orbit", "search", "similar", "ORB-1"]);
        match cli.command {
            Commands::Search(args) => {
                assert_eq!(args.query, None);
                match args.command {
                    Some(SearchSubcommand::Similar(similar)) => {
                        assert_eq!(similar.id, "ORB-1");
                    }
                    _ => panic!("expected search similar"),
                }
            }
            _ => panic!("expected top-level search command"),
        }
    }

    #[test]
    fn cli_parses_top_level_search_path_lookup() {
        let cli = Cli::parse_from(["orbit", "search", "path", "crates/orbit-cli/"]);
        match cli.command {
            Commands::Search(args) => {
                assert_eq!(args.query, None);
                match args.command {
                    Some(SearchSubcommand::Path(path)) => {
                        assert_eq!(path.path, "crates/orbit-cli/");
                    }
                    _ => panic!("expected search path"),
                }
            }
            _ => panic!("expected top-level search command"),
        }
    }

    #[test]
    fn cli_parses_top_level_search_tag_filter() {
        let cli = Cli::parse_from(["orbit", "search", "perf", "--tag", "perf", "--kind", "adr"]);
        match cli.command {
            Commands::Search(args) => {
                assert_eq!(args.query.as_deref(), Some("perf"));
                assert_eq!(args.tags, vec!["perf"]);
                assert_eq!(args.kind, SearchKindArg::Adr);
            }
            _ => panic!("expected top-level search command"),
        }
    }

    #[test]
    fn cli_rejects_search_query_with_semantic_neighbor() {
        assert!(Cli::try_parse_from(["orbit", "search", "query", "ORB-1"]).is_err());
    }

    #[test]
    fn cli_rejects_search_related_flag() {
        let legacy_flag = concat!("--", "related");
        assert!(Cli::try_parse_from(["orbit", "search", legacy_flag, "ORB-1"]).is_err());
    }

    #[test]
    fn cli_rejects_search_semantic_flag() {
        assert!(Cli::try_parse_from(["orbit", "search", "--semantic", "ORB-1"]).is_err());
    }

    #[test]
    fn cli_rejects_retired_search_field_and_model_flags() {
        assert!(Cli::try_parse_from(["orbit", "search", "query", "--field", "title"]).is_err());
        assert!(Cli::try_parse_from(["orbit", "search", "query", "--model", "bge-small"]).is_err());
        assert!(
            Cli::try_parse_from(["orbit", "search", "similar", "ORB-1", "--field", "title"])
                .is_err()
        );
        assert!(
            Cli::try_parse_from(["orbit", "search", "path", "crates/", "--model", "bge-small"])
                .is_err()
        );
    }

    #[test]
    fn cli_rejects_retired_search_path_flag() {
        assert!(Cli::try_parse_from(["orbit", "search", "--path", "crates/"]).is_err());
    }

    #[test]
    fn cli_rejects_top_level_serve() {
        assert!(Cli::try_parse_from(["orbit", "serve"]).is_err());
    }

    #[test]
    fn cli_rejects_down_alias() {
        assert!(Cli::try_parse_from(["orbit", "mcp", "down"]).is_err());
    }
}
