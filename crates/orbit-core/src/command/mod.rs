//! Command implementations for all Orbit CLI subcommands.
//!
//! Each sub-module (task, job, activity, skill, audit, tool, init)
//! provides the data types and logic for one command group. Commands are
//! executed via the `Execute` trait, which receives an `&OrbitRuntime` and
//! produces an `OrbitError` on failure.
//!
//! The `init` module is special: it also provides `execute_without_runtime`
//! for bootstrapping a new Orbit root before a runtime can be constructed.
//! Default YAML assets (e.g., sample skills, config templates) are embedded
//! at compile time via `include_str!` and seeded to disk on first `orbit init`.

/// Audit identity used for system-initiated (non-agent) mutations.
/// `pub` because the direct v2 activity runner moved to `orbit-cmd`
/// [ORB-10016] and stamps the same identity.
pub const SYSTEM_AUDIT_IDENTITY: &str = "system";

pub mod activity;
pub mod audit_event;
pub mod backend_resolver;
pub mod docs;
pub mod executor;
pub mod gc;
pub mod gc_audit;
pub mod init;
pub mod job;
pub mod learning;
pub mod pipeline_run;
pub mod policy;
pub mod review_thread_hook;
pub mod routine;
pub mod search;
pub mod semantic;
pub mod skill;
pub mod skill_gc;
pub mod skill_ownership;
pub mod task;
pub mod task_migration;
pub mod tool;
pub mod workflow;

#[cfg(test)]
mod tests;
