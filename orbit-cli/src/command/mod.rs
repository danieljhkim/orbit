pub mod activity;
pub mod audit;
pub mod config;
pub mod init;
pub mod job;
pub mod job_run;
pub mod skill;
pub mod task;
pub mod tool;

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
    Audit(audit::AuditCommand),
    Activity(activity::ActivityCommand),
    Skill(skill::SkillCommand),
    Job(job::JobCommand),
    JobRun(job_run::JobRunCommand),
}

impl Execute for Commands {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            Commands::Config(cmd) => cmd.execute(runtime),
            Commands::Init(cmd) => cmd.execute(runtime),
            Commands::Tool(cmd) => cmd.execute(runtime),
            Commands::Task(cmd) => cmd.execute(runtime),
            Commands::Audit(cmd) => cmd.execute(runtime),
            Commands::Activity(cmd) => cmd.execute(runtime),
            Commands::Skill(cmd) => cmd.execute(runtime),
            Commands::Job(cmd) => cmd.execute(runtime),
            Commands::JobRun(cmd) => cmd.execute(runtime),
        }
    }
}
