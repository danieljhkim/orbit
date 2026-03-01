pub mod agent;
pub mod audit;
pub mod config;
pub mod init;
pub mod job;
pub mod mcp;
pub mod skill;
pub mod task;
pub mod tool;
pub mod watch;
pub mod work;

use clap::{Parser, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

pub trait Execute {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError>;
}

#[derive(Parser)]
#[command(name = "orbit")]
#[command(about = "Orbit v2.1 CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Config(config::ConfigCommand),
    Init(init::InitCommand),
    Tool(tool::ToolCommand),
    Task(task::TaskCommand),
    Agent(agent::AgentCommand),
    Audit(audit::AuditCommand),
    Work(work::WorkCommand),
    Skill(skill::SkillCommand),
    Job(job::JobCommand),
    Watch(watch::WatchCommand),
    Mcp(mcp::McpCommand),
}

impl Execute for Commands {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            Commands::Config(cmd) => cmd.execute(runtime),
            Commands::Init(cmd) => cmd.execute(runtime),
            Commands::Tool(cmd) => cmd.execute(runtime),
            Commands::Task(cmd) => cmd.execute(runtime),
            Commands::Agent(cmd) => cmd.execute(runtime),
            Commands::Audit(cmd) => cmd.execute(runtime),
            Commands::Work(cmd) => cmd.execute(runtime),
            Commands::Skill(cmd) => cmd.execute(runtime),
            Commands::Job(cmd) => cmd.execute(runtime),
            Commands::Watch(cmd) => cmd.execute(runtime),
            Commands::Mcp(cmd) => cmd.execute(runtime),
        }
    }
}
