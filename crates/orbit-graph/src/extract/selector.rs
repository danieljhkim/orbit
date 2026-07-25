//! Selector parsing and filesystem-anchor helpers for graph queries.
//!
//! The canonical implementation lives in `orbit_common::utility::selector`
//! (consolidated in ORB-10011; see ADR-0202). This module re-exports it so
//! graph consumers keep addressing the stable selector grammar through
//! `orbit_graph::extract::selector` / `orbit_graph::Selector`.

pub use orbit_common::utility::selector::{Selector, SelectorParseError};
