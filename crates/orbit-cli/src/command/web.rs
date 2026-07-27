//! `orbit web` — thin delegator to the orbit-dashboard crate.
//!
//! The real implementation (ServeArgs, serve(), router, assets, API handlers)
//! lives in the `orbit-dashboard` crate so that orbit-cli incremental builds
//! do not pay the axum dependency tax on every change.

use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

/// Thin wrapper so `orbit web` continues to work and audit events stay stable.
#[derive(Args)]
#[command(
    about = "Run the Orbit dashboard",
    arg_required_else_help = true,
    subcommand_required = true
)]
pub struct WebCommand {
    #[command(subcommand)]
    pub command: WebSubcommand,
}

impl Execute for WebCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum WebSubcommand {
    /// Run the Orbit dashboard
    Serve(orbit_dashboard::ServeArgs),
    /// Open a remote workspace's dashboard over an SSH tunnel
    Connect(orbit_dashboard::ConnectArgs),
}

impl Execute for WebSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            WebSubcommand::Serve(args) => orbit_dashboard::serve(runtime, args),
            // `connect` is a client-side tunnel helper; the workspace lives on
            // the remote, so it needs no local runtime.
            WebSubcommand::Connect(args) => orbit_dashboard::connect(args),
        }
    }
}
