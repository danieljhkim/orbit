//! Selector parsing and filesystem-anchor helpers for graph queries.
//!
//! The canonical implementation lives in `orbit_common::utility::selector`
//! (consolidated in ORB-10011; see ADR-0202). This module re-exports it so
//! graph consumers keep addressing the stable selector grammar through
//! `orbit_graph_extract::selector` / `orbit_graph_extract::Selector`.

pub use orbit_common::utility::selector::{
    Selector, SelectorLookupKey, SelectorParseError, anchor_path, canonical_selector,
    canonical_selector_in_workspace, exists_in_workspace, overlaps, selector_error_to_orbit,
    shared_anchor_prefix_depth,
};
