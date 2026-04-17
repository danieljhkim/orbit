use std::collections::{BTreeMap, HashMap};

use crate::graph::navigator::GraphNodeRef;

/// A single structured search result.
pub struct SearchResult {
    pub selector: String,
    pub name: String,
    /// `"dir"`, `"file"`, or a leaf kind like `"function"`, `"struct"`.
    pub kind: String,
    /// The containing file path (populated for file and leaf nodes).
    pub file: Option<String>,
}

/// Aggregate overview of a (scoped) graph.
pub struct GraphOverview {
    pub total_dirs: usize,
    pub total_files: usize,
    pub total_symbols: usize,
    pub languages: HashMap<String, usize>,
    pub symbol_kinds: HashMap<String, usize>,
    pub files: Vec<FileOverview>,
}

pub struct FileOverview {
    pub selector: String,
    pub path: String,
    pub name: String,
    pub symbol_count: usize,
    pub symbols: Vec<SymbolBrief>,
}

pub struct SymbolBrief {
    pub name: String,
    pub kind: String,
    pub selector: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphOverviewSummary {
    pub total_dirs: usize,
    pub total_files: usize,
    pub total_symbols: usize,
    pub languages: HashMap<String, usize>,
    pub symbol_kinds: HashMap<String, usize>,
    pub dir_file_counts: BTreeMap<String, usize>,
    pub top_files: Vec<TopFileEntry>,
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopFileEntry {
    pub selector: String,
    pub name: String,
    pub symbol_count: usize,
}

/// A reference hit from `find_references`.
pub struct ReferenceHit {
    pub selector: String,
    pub name: String,
    pub file: String,
    pub kind: String,
}

/// Bounded context around a single node.
pub struct NodeContext<'a> {
    pub node: GraphNodeRef<'a>,
    pub lineage: Vec<GraphNodeRef<'a>>,
    pub siblings: Vec<GraphNodeRef<'a>>,
    pub children: Vec<GraphNodeRef<'a>>,
}
