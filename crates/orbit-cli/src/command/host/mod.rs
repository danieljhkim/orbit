//! v1 host administration exposes only a machine-local rename. The fleet
//! register/list/retire implementations remain in this directory as dormant
//! v2 substrate; they are intentionally not module-linked. See
//! `docs/design/host-registry/2_design.md` §2.1 (ADR-0358).

mod command;
mod rename;
// `cfg(any())` is the explicit dormant boundary: keep the reviewed v2 source
// connected to the module tree for orphan-file auditing without compiling a
// callable v1 surface.
#[cfg(any())]
mod register;
#[cfg(any())]
mod retire;

pub use command::{HostCommand, HostSubcommand};

// Preserve the dormant fleet-list renderer coverage without linking the
// command into production. This test-only module is also a compile-visible
// seam proving the fleet path is absent from v1 builds.
#[cfg(test)]
mod list;
#[cfg(test)]
mod tests;
