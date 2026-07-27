//! Built-in deterministic automation actions for the Orbit engine.
//!
//! The v1 executor stack — the `ActivityExecutor` trait, its registry, and the
//! `direct_agent` / `external` / `cli_command` implementations — was deleted in
//! [ORB-10395]; v2 dispatch in [`crate::activity_job`] is the only execution
//! path. What survives here is [`automation::execute_action`], the deterministic
//! action dispatcher that v2 job steps call directly.

pub mod automation;
