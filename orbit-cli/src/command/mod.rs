pub mod agent;
pub mod audit;
pub mod config;
pub mod init;
pub mod job;
pub mod scheduler;
pub mod skill;
pub mod task;
pub mod tool;
pub mod watch;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

pub trait Execute {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError>;
}

#[derive(Parser)]
#[command(name = "orbit")]
#[command(about = "Orbit v2.1 CLI")]
pub struct Cli {
    /// Override the Orbit root directory (highest precedence)
    #[arg(long, global = true)]
    pub root: Option<PathBuf>,

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
    Job(job::JobCommand),
    Skill(skill::SkillCommand),
    Scheduler(scheduler::SchedulerCommand),
    Watch(watch::WatchCommand),
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
            Commands::Job(cmd) => cmd.execute(runtime),
            Commands::Skill(cmd) => cmd.execute(runtime),
            Commands::Scheduler(cmd) => cmd.execute(runtime),
            Commands::Watch(cmd) => cmd.execute(runtime),
        }
    }
}
