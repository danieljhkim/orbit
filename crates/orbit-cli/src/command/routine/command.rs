//! `orbit routine` — inspect and control routines on this host [ORB-10021].
//!
//! Every subcommand operates on the global registry and host-local scheduler
//! state, so the whole parent command dispatches without a workspace runtime
//! (see `main.rs`), like `orbit sweep` itself.

use clap::{Args, Subcommand};
use orbit_core::OrbitError;

use super::init::RoutineInitArgs;
use super::list::RoutineListArgs;
use super::pause::RoutinePauseArgs;
use super::resume::RoutineResumeArgs;
use super::show::RoutineShowArgs;

#[derive(Args)]
#[command(
    about = "Inspect and control scheduled routines on this host",
    arg_required_else_help = true,
    subcommand_required = true,
    after_help = "Routine definitions are versioned YAML under `.orbit/routines/` in\n\
                  workspaces with `[routines] role = \"source\"`. Pauses are host-local\n\
                  and never synced. The scheduler pass itself is `orbit sweep`."
)]
pub struct RoutineCommand {
    #[command(subcommand)]
    pub command: RoutineSubcommand,
}

impl RoutineCommand {
    /// All routine subcommands resolve state from the global root; none may
    /// bootstrap a workspace from the caller's cwd.
    pub fn execute_without_runtime(self) -> Result<(), OrbitError> {
        self.command.execute_without_runtime()
    }
}

#[derive(Subcommand)]
pub enum RoutineSubcommand {
    /// List every routine with toggles, next-due, and last fire
    List(RoutineListArgs),
    /// Show one routine's definition, effective state, and recent fires
    Show(RoutineShowArgs),
    /// Suppress a routine on this host (host-local, survives reboots)
    Pause(RoutinePauseArgs),
    /// Clear a host-local pause
    Resume(RoutineResumeArgs),
    /// Set this host's identity and optionally install the OS clock unit
    Init(RoutineInitArgs),
}

impl RoutineSubcommand {
    fn execute_without_runtime(self) -> Result<(), OrbitError> {
        match self {
            Self::List(args) => args.execute_without_runtime(),
            Self::Show(args) => args.execute_without_runtime(),
            Self::Pause(args) => args.execute_without_runtime(),
            Self::Resume(args) => args.execute_without_runtime(),
            Self::Init(args) => args.execute_without_runtime(),
        }
    }
}
