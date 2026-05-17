pub mod adr;
pub mod definitions;
pub mod design;
pub mod environment;
pub mod learning;
pub mod log;
pub mod mcp;
pub mod observe;
pub mod run;
pub mod semantic;
pub mod task;
pub mod web;

pub use definitions::{activity, executor, job, policy, skill, tool};
pub use environment::{config, init, workspace};
pub use observe::{audit, graph, metrics, scoreboard};

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

Operate:
  run         Run a workflow (ship, duel-plan, job)
  task        Create, update, and manage tasks
  adr         Architecture Decision Record operations
  design      Design doc operations
  learning    Create, search, and curate project learnings
  semantic    Manage local semantic-search indexing

Observe:
  graph       Query the knowledge graph
  audit       Query the audit event log
  log         Tail the unified Orbit log feed
  metrics     Show metrics
  scoreboard  Show scoreboards (duel-plan, PR, task review)

Definitions:
  activity    View activity definitions
  job         View job definitions
  tool        View tool registry
  policy      View filesystem policies
  executor    View executors

Services:
  mcp         Register MCP client integrations and run the MCP server
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

    // ── Operate ──
    Run(run::RunCommand),
    Task(Box<task::TaskCommand>),
    Adr(adr::AdrCommand),
    Design(design::DesignCommand),
    Learning(learning::LearningCommand),
    Semantic(semantic::SemanticCommand),

    // ── Observe ──
    Graph(graph::GraphCommand),
    Audit(audit::AuditCommand),
    Log(log::LogCommand),
    Metrics(metrics::MetricsCommand),
    Scoreboard(scoreboard::ScoreboardCommand),

    // ── Definitions ──
    Activity(activity::ActivityCommand),
    Job(job::JobCommand),
    Tool(tool::ToolCommand),
    Policy(policy::PolicyCommand),
    Executor(executor::ExecutorCommand),

    // ── Services ──
    Mcp(mcp::McpCommand),
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
            Commands::Run(cmd) => cmd.execute(runtime),
            Commands::Task(cmd) => (*cmd).execute(runtime),
            Commands::Adr(cmd) => cmd.execute(runtime),
            Commands::Design(cmd) => cmd.execute(runtime),
            Commands::Learning(cmd) => cmd.execute(runtime),
            Commands::Semantic(cmd) => cmd.execute(runtime),
            Commands::Graph(cmd) => cmd.execute(runtime),
            Commands::Audit(cmd) => cmd.execute(runtime),
            Commands::Log(cmd) => cmd.execute(runtime),
            Commands::Metrics(cmd) => cmd.execute(runtime),
            Commands::Scoreboard(cmd) => cmd.execute(runtime),
            Commands::Activity(cmd) => cmd.execute(runtime),
            Commands::Job(cmd) => cmd.execute(runtime),
            Commands::Tool(cmd) => cmd.execute(runtime),
            Commands::Policy(cmd) => cmd.execute(runtime),
            Commands::Executor(cmd) => cmd.execute(runtime),
            Commands::Mcp(cmd) => cmd.execute(runtime),
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
        Cli, Commands, design::DesignSubcommand, mcp::McpSubcommand, semantic::SemanticSubcommand,
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
    fn cli_parses_semantic_search() {
        let cli = Cli::parse_from(["orbit", "semantic", "search", "semantic search design"]);
        match cli.command {
            Commands::Semantic(command) => match command.command {
                SemanticSubcommand::Search(args) => {
                    assert_eq!(args.query, "semantic search design")
                }
                _ => panic!("expected semantic search"),
            },
            _ => panic!("expected top-level semantic command"),
        }
    }

    #[test]
    fn cli_parses_semantic_related() {
        let cli = Cli::parse_from(["orbit", "semantic", "related", "T20260510-3"]);
        match cli.command {
            Commands::Semantic(command) => match command.command {
                SemanticSubcommand::Related(args) => assert_eq!(args.task_id, "T20260510-3"),
                _ => panic!("expected semantic related"),
            },
            _ => panic!("expected top-level semantic command"),
        }
    }

    #[test]
    fn cli_parses_design_check() {
        let cli = Cli::parse_from(["orbit", "design", "check", "--warn-only"]);
        match cli.command {
            Commands::Design(command) => match command.command {
                DesignSubcommand::Check(args) => assert!(args.warn_only),
            },
            _ => panic!("expected top-level design command"),
        }
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
