//! Temporary compatibility surface for the satellite registry cache.
//!
//! The domain implementation moved to `orbit-registry` in ORB-10302.

pub use orbit_registry::registry_cache::*;

#[cfg(test)]
#[path = "tests/registry_cache.rs"]
mod tests;
