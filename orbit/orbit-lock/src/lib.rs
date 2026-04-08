//! SQLite-backed file lock coordination for Orbit's mutating tool surface.
//!
//! This crate provides the small persistence layer that prevents multiple tasks
//! from editing the same workspace path at the same time. The lock store is
//! intentionally narrow: Orbit uses it to guard fs tool writes while higher
//! layers keep task lifecycle and policy concerns elsewhere.
//!
//! # Role
//! Depends only on `orbit-types` plus SQLite support crates. Consumed by
//! `orbit-tools` and `orbit-core` when they need shared write-conflict checks
//! for task-scoped file mutations.
//!
//! # Key exports
//! - [`FileLockChecker`] - trait used by fs tools to validate and auto-acquire locks
//! - [`FileLockStore`] - SQLite-backed implementation for file-lock coordination
//! - [`FileLock`] / [`FileLockConflict`] - serialized lock metadata and conflict details
//! - [`apply_lock_schema`] - installs the required SQLite schema
//!
//! # Dependency direction
//! `orbit-types` -> `orbit-lock` -> orbit-tools, orbit-core

mod schema;
mod store;

pub use schema::apply_lock_schema;
pub use store::{FileLock, FileLockChecker, FileLockConflict, FileLockStore};
