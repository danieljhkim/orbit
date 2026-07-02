//! Test-only allowlist: the original tests under orbit-cli passed the same lints via
//! the crate-level test harness configuration; duplicated here for the extracted crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod test_support;

mod adrs;
mod denials;
mod diagnostics;
mod frictions;
mod handlers;
mod learnings;
mod log;
mod metrics;
mod review_threads;
mod runs;
mod scoreboard;
mod tasks;
mod workspaces;
