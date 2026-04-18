//! Command implementations for all Orbit CLI subcommands.
//!
//! Each sub-module (task, job, activity, skill, audit, tool, init, job_run)
//! provides the data types and logic for one command group. Commands are
//! executed via the `Execute` trait, which receives an `&OrbitRuntime` and
//! produces an `OrbitError` on failure.
//!
//! The `init` module is special: it also provides `execute_without_runtime`
//! for bootstrapping a new Orbit root before a runtime can be constructed.
//! Default YAML assets (e.g., sample skills, config templates) are embedded
//! at compile time via `include_str!` and seeded to disk on first `orbit init`.

pub mod activity;
pub mod activity_v2;
pub mod audit_event;
pub mod diagnostics;
pub mod executor;
pub mod init;
pub mod job;
pub mod job_run;
pub mod policy;
pub mod reconcile;
pub mod skill;
pub mod task;
pub mod task_template;
pub mod tool;
pub mod workflow;
