#![deny(clippy::print_stderr, clippy::print_stdout)]
// ORB-00004: legacy domain-schema surfaces still need a focused documentation pass.
#![allow(missing_docs)]
// Unit tests use unwrap/expect for fixture setup; production call sites remain linted.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_html_tags,
    rustdoc::private_intra_doc_links
)]
//! Shared leaf crate for the Orbit workspace.
//!
//! The public surface is intentionally split into four namespaces:
//! - [`authorization`] for the capability chokepoint every entry surface asks
//!   before performing a governed operation (ORB-10453)
//! - [`friction`] for shared friction taxonomy defaults and the friction
//!   operation registry
//! - [`migration`] for forward-only schema migrations of YAML artifacts
//! - [`operation`] for the operations-as-data kernel every noun registry is
//!   declared in (ADR-0209 bearing 1)
//! - [`types`] for Orbit domain types, `OrbitError`, IDs, and the v2 schemas
//! - [`utility`] for generic helpers like filesystem, redaction, logging,
//!   and blob storage
//! - [`tracing`] as the shared structured-event facade used by Orbit crates

pub mod authorization;
pub mod friction;
pub mod migration;
pub mod model_defaults;
pub mod operation;
pub mod types;
pub mod utility;

/// Scoped process-environment guards for tests that assert on the absence of
/// Orbit run/identity context. Behind the `test-util` feature.
#[cfg(any(test, feature = "test-util"))]
pub mod test_env;

/// Frozen model-name constants shared by tests across the workspace. Behind the
/// `test-util` feature so integration tests in sibling crates can reach it.
#[cfg(any(test, feature = "test-util"))]
pub mod test_fixtures;

/// Re-export Orbit's tracing facade for crates that already depend on
/// `orbit-common` and need to emit structured events without expanding their
/// dependency surface.
pub use tracing;
