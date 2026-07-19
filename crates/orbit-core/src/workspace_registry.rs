//! Temporary compatibility surface for the local workspace registry.
//!
//! The domain implementation moved to `orbit-remote` in ORB-10302.

pub use orbit_remote::workspace_registry::*;

#[cfg(test)]
#[path = "tests/workspace_registry.rs"]
mod tests;
