//! File-backed scoreboard implementations.
//!
//! This directory module keeps the scoreboard implementation split into
//! focused files while preserving the existing `file::scoreboard::*` paths.

pub(crate) mod common;
pub(crate) mod duel_scoreboard;
pub(crate) mod planning_duel_scoreboard;
pub(crate) mod pr_scoreboard;
pub(crate) mod scoreboard_summary;
pub(crate) mod token_scoreboard;
