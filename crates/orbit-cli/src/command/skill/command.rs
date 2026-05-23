use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::doctor::SkillDoctorArgs;
use super::link::SkillLinkArgs;
use super::list::SkillListArgs;
use super::show::SkillShowArgs;
use super::unlink::SkillUnlinkArgs;

#[derive(Args)]
#[command(about = "Manage agent skill definitions")]
pub struct SkillCommand {
    #[command(subcommand)]
    pub command: SkillSubcommand,
}

impl Execute for SkillCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum SkillSubcommand {
    /// List all available skills
    List(SkillListArgs),
    /// Show detailed information about a skill
    Show(SkillShowArgs),
    /// Validate skill health and configuration
    Doctor(SkillDoctorArgs),
    /// Re-create skill symlinks in ~/.agents/skills/ and ~/.claude/skills/
    Link(SkillLinkArgs),
    /// Remove skill symlinks from ~/.agents/skills/ and ~/.claude/skills/
    Unlink(SkillUnlinkArgs),
}

impl Execute for SkillSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            SkillSubcommand::List(args) => args.execute(runtime),
            SkillSubcommand::Show(args) => args.execute(runtime),
            SkillSubcommand::Doctor(args) => args.execute(runtime),
            SkillSubcommand::Link(args) => args.execute(runtime),
            SkillSubcommand::Unlink(args) => args.execute(runtime),
        }
    }
}
