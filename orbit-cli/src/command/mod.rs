pub mod agent;
pub mod audit;
pub mod config;
pub mod entry;
pub mod job;
pub mod skill;
pub mod task;
pub mod tool;
pub mod watch;

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
    Tool(tool::ToolCommand),
    Task(task::TaskCommand),
    Agent(agent::AgentCommand),
    Audit(audit::AuditCommand),
    Entry(entry::EntryCommand),
    Skill(skill::SkillCommand),
    Job(job::JobCommand),
    Watch(watch::WatchCommand),
}

impl Execute for Commands {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            Commands::Tool(cmd) => cmd.execute(runtime),
            Commands::Task(cmd) => cmd.execute(runtime),
            Commands::Agent(cmd) => cmd.execute(runtime),
            Commands::Audit(cmd) => cmd.execute(runtime),
            Commands::Entry(cmd) => cmd.execute(runtime),
            Commands::Skill(cmd) => cmd.execute(runtime),
            Commands::Job(cmd) => cmd.execute(runtime),
            Commands::Watch(cmd) => cmd.execute(runtime),
        }
    }
}
