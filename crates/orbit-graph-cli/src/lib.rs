#![allow(missing_docs)]

//! Library surface for the Orbit graph JSON CLI.
//!
//! The clap [`Command`] subcommand enum and its [`Command::run`] dispatch are
//! shared by two front ends: the standalone `orbit-graph-cli` binary
//! (`src/main.rs`) and the `orbit graph` subcommand embedded in `orbit-cli`.
//! Keeping a single command layer here means both surfaces stay in lockstep
//! without duplicating the per-query argument structs.

mod commands;

pub use commands::{Cli, CliError, Command};
