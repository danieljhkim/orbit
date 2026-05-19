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

pub(crate) const SYSTEM_AUDIT_IDENTITY: &str = "system";

pub mod activity;
pub mod activity_v2;
pub mod adr_migration;
pub mod agent_rules;
pub mod audit_event;
pub mod backend_resolver;
pub mod design;
pub mod diagnostics;
pub mod docs;
pub mod executor;
pub mod graph;
pub mod init;
pub mod job;
pub mod learning;
pub mod learning_hook;
pub mod pipeline_run;
pub mod policy;
pub mod semantic;
pub mod skill;
pub mod task;
pub mod task_template;
pub mod tool;
pub mod workflow;
