//! Shared cross-crate utilities for the Orbit workspace.
//!
//! Scope: generic helpers with no Orbit domain knowledge. What belongs here:
//! - atomic filesystem primitives (`fs`)
//! - secret redaction — env values, HTTP headers, argv tokens (`redaction`)
//! - logging subscriber setup (`logging`)
//!
//! What does NOT belong here: anything that knows about tasks, activities,
//! jobs, policies, the knowledge graph, or on-disk layouts of those. Those
//! stay in their owning crates. Domain types live in `orbit-types`.

pub mod blob_store;
pub mod fs;
pub mod logging;
pub mod redaction;
