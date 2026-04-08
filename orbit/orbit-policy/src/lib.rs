//! RBAC policy evaluation engine for Orbit tool and command authorization.
//!
//! Evaluates whether a given actor (identified by [`orbit_types::Role`]) is
//! permitted to invoke a specific tool or CLI entrypoint. Admin callers bypass
//! explicit tool deny rules, while agent callers are subject to them; both
//! roles still respect the engine's default allow or default deny mode.
//!
//! # Role
//! Sits directly above `orbit-types` in the dependency graph. Consumed by
//! `orbit-core`, which wires the [`PolicyEngine`] into the runtime so that
//! every tool call passes through policy evaluation before execution.
//!
//! # Key exports
//! - [`PolicyEngine`] — stateful evaluator; constructed with allow/deny rule sets
//! - [`PolicyContext`] — per-call context (entrypoint, tool name, role)
//! - [`PolicyDecision`] — `Allow` or `Deny { reason }` result
//!
//! # Dependency direction
//! `orbit-types` → `orbit-policy` → orbit-core

pub mod decision;
pub mod engine;
mod evaluator;

pub use decision::PolicyDecision;
pub use engine::{PolicyContext, PolicyEngine};
