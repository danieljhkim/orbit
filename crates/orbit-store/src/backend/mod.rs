//! Backend trait abstraction and layered store pattern for Orbit persistence.
//!
//! Defines the store backend traits for each persistence concern and provides
//! explicit builders for workspace-only, global-only, and split-scope stores.

mod contracts;
mod factory;
mod file_backends;
mod sqlite_backends;

pub use contracts::*;
pub use factory::*;
