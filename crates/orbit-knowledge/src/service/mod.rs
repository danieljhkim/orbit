//! Service layer: selector resolution, search, and context pack rendering.

pub mod lineage;

mod context;
mod overview;
mod search;
mod selectors;
mod types;

use std::collections::HashMap;

use crate::graph::navigator::GraphNavigator;
use crate::graph::nodes::CodebaseGraphV1;

pub use overview::compact_from_overview;
pub use types::{
    FileOverview, GraphOverview, GraphOverviewSummary, NodeContext, ReferenceHit, SearchResult,
    SymbolBrief, TopFileEntry,
};

/// High-level query service over a [`CodebaseGraphV1`].
///
/// Wraps a [`GraphNavigator`] and adds selector resolution, search,
/// and context-building helpers.
pub struct GraphContextService<'a> {
    graph: &'a CodebaseGraphV1,
    nav: GraphNavigator<'a>,
    /// location key → node ID (for selector resolution).
    /// Dirs/files use location as-is; leaves use "location:kind" to
    /// disambiguate struct vs impl at the same qualified name.
    location_index: HashMap<String, &'a str>,
}
