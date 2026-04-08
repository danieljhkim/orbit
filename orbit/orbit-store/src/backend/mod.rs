//! Backend trait abstraction and layered store pattern for Orbit persistence.
//!
//! Defines the store backend traits (`TaskStoreBackend`, `JobStoreBackend`,
//! `ActivityStoreBackend`, `AuditEventStoreBackend`, `ToolStoreBackend`) that
//! all persistence implementations must satisfy. Also provides factory functions
//! that construct either a raw file/SQLite backend or a layered (resolved) store
//! that merges a global root store with a workspace-local store according to a
//! [`ResolvedScope`] strategy.
//!
//! The layered pattern means callers always work through the same trait interface
//! regardless of whether scoping is active; the factory picks the right wrapper.

mod contracts;
mod factory;
mod file_backends;
mod layered_activity;
mod layered_job;
mod sqlite_backends;

pub use contracts::*;
pub use factory::*;
pub use layered_activity::LayeredActivityStore;
pub use layered_job::LayeredJobStore;
