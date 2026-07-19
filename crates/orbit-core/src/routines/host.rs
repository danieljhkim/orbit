//! Temporary compatibility surface for host identity.
//!
//! The domain implementation moved to `orbit-remote` in ORB-10302. Existing
//! `orbit-core::routines` callers keep compiling through this explicit re-export.

pub use orbit_remote::host_identity::*;
