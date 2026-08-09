//! `orbit adr ...` CLI surface.
//!
//! ORB-00289: `orbit.adr.list` was inactivated on the agent MCP surface
//! (agents discover ADRs via `orbit search --kind adr`). The CLI/admin
//! path still needs a way to list ADRs — including features like
//! `--include-remote` that don't surface through `orbit search` — so that
//! subcommand reaches the underlying tool through `runtime.run_tool`,
//! which bypasses `ensure_tool_agent_facing` while preserving the tool's
//! input parsing and filter semantics.
//!
//! ORB-10479: `orbit.adr.restore` (ORB-10538) is registered inactive for the
//! same reason — exact-id repair is an operator surface, not an agent one.
//! Registering it inactive also puts it out of reach of `orbit tool run`,
//! which gates on `ensure_tool_agent_facing`, so the tool needs its
//! subcommand to be reachable at all.
//!
//! ORB-10668: `add` / `update` / `supersede` complete the authoring and
//! lifecycle half of the surface. Every one of them delegates to the
//! matching `orbit.adr.*` tool, which stays the single implementation of ADR
//! semantics — ID allocation, the `proposed -> accepted` rules, and the
//! `artifact_not_local` federation guard all live there, not here.

use clap::{Args, Subcommand};
use orbit_core::OrbitRuntime;

use crate::command::{CommandOut, Execute};

use super::add::AdrAddArgs;
use super::list::AdrListArgs;
use super::reconcile::AdrReconcileArgs;
use super::restore::AdrRestoreArgs;
use super::show::AdrShowArgs;
use super::supersede::AdrSupersedeArgs;
use super::update::AdrUpdateArgs;

#[derive(Args)]
#[command(
    about = "Author, inspect, and move Architecture Decision Records through their lifecycle"
)]
pub struct AdrCommand {
    #[command(subcommand)]
    pub command: AdrSubcommand,
}

#[derive(Subcommand)]
pub enum AdrSubcommand {
    /// Create a Proposed ADR and print its newly allocated ID
    Add(AdrAddArgs),
    /// List ADRs with optional filters
    List(AdrListArgs),
    /// Show one ADR, including its body and artifact origin
    Show(AdrShowArgs),
    /// Update mutable ADR fields and perform guarded status transitions
    /// (`--status accepted` needs a related task; supersession has its own verb)
    Update(AdrUpdateArgs),
    /// Mark an ADR superseded by an already-accepted replacement
    Supersede(AdrSupersedeArgs),
    /// Restore an unreadable ADR at its exact existing allocation
    Restore(AdrRestoreArgs),
    /// Reconcile a federated ADR bundle into the current checkout
    Reconcile(AdrReconcileArgs),
}

impl Execute for AdrCommand {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        self.command.execute(runtime)
    }
}

impl Execute for AdrSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        match self {
            AdrSubcommand::Add(args) => args.execute(runtime),
            AdrSubcommand::List(args) => args.execute(runtime),
            AdrSubcommand::Show(args) => args.execute(runtime),
            AdrSubcommand::Update(args) => args.execute(runtime),
            AdrSubcommand::Supersede(args) => args.execute(runtime),
            AdrSubcommand::Restore(args) => args.execute(runtime),
            AdrSubcommand::Reconcile(args) => args.execute(runtime),
        }
    }
}
