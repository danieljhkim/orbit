use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BinaryHeap, HashMap};

use crate::graph::navigator::GraphNodeRef;

use super::{
    FileOverview, GraphContextService, GraphOverview, GraphOverviewSummary, SymbolBrief,
    TopFileEntry,
};

impl<'a> GraphContextService<'a> {
    /// Build an aggregate overview of the graph, optionally scoped by location prefix.
    pub fn overview(&self, location_prefix: Option<&str>) -> GraphOverview {
        let in_scope = |location: &str| {
            location_prefix
                .map(|prefix| location.starts_with(prefix))
                .unwrap_or(true)
        };

        let mut total_dirs = 0usize;
        let mut total_files = 0usize;
        let mut total_symbols = 0usize;
        let mut languages: HashMap<String, usize> = HashMap::new();
        let mut symbol_kinds: HashMap<String, usize> = HashMap::new();
        let mut file_overviews = Vec::new();

        for dir in &self.graph.dirs {
            if in_scope(&dir.base.location) {
                total_dirs += 1;
            }
        }

        let mut file_leaves: HashMap<&str, Vec<SymbolBrief>> = HashMap::new();
        for leaf in &self.graph.leaves {
            if !in_scope(&leaf.base.location) {
                continue;
            }
            total_symbols += 1;
            let kind = leaf.kind.to_string();
            *symbol_kinds.entry(kind.clone()).or_default() += 1;
            let file_path = leaf
                .base
                .location
                .split_once('#')
                .map(|(path, _)| path)
                .unwrap_or(&leaf.base.location);
            file_leaves.entry(file_path).or_default().push(SymbolBrief {
                name: leaf.base.name.clone(),
                kind,
                selector: self.selector_for_node(GraphNodeRef::Leaf(leaf)),
            });
        }

        for file in &self.graph.files {
            if !in_scope(&file.base.location) {
                continue;
            }

            total_files += 1;
            if !file.base.language.is_empty() {
                *languages.entry(file.base.language.clone()).or_default() += 1;
            }

            let symbols = file_leaves
                .remove(file.base.location.as_str())
                .unwrap_or_default();
            file_overviews.push(FileOverview {
                selector: self.selector_for_node(GraphNodeRef::File(file)),
                path: file.base.location.clone(),
                name: file.base.name.clone(),
                symbol_count: symbols.len(),
                symbols,
            });
        }

        GraphOverview {
            total_dirs,
            total_files,
            total_symbols,
            languages,
            symbol_kinds,
            files: file_overviews,
        }
    }
}

impl GraphOverview {
    /// Select the most symbol-dense files with deterministic tie-breaking.
    pub fn top_files(&self, limit: usize) -> Vec<TopFileEntry> {
        if limit == 0 {
            return Vec::new();
        }

        let heap_capacity = limit.min(self.files.len()).saturating_add(1);
        let mut heap: BinaryHeap<Reverse<FileOverviewRank<'_>>> =
            BinaryHeap::with_capacity(heap_capacity);
        for file in &self.files {
            heap.push(Reverse(FileOverviewRank(file)));
            if heap.len() > limit {
                heap.pop();
            }
        }

        let mut files: Vec<&FileOverview> = heap
            .into_iter()
            .map(|Reverse(FileOverviewRank(file))| file)
            .collect();
        files.sort_by(|left, right| compare_file_overview_rank(left, right));
        files
            .into_iter()
            .map(FileOverview::top_file_entry)
            .collect()
    }
}

#[derive(Clone, Copy)]
struct FileOverviewRank<'a>(&'a FileOverview);

impl PartialEq for FileOverviewRank<'_> {
    fn eq(&self, other: &Self) -> bool {
        compare_file_overview_rank(self.0, other.0) == Ordering::Equal
    }
}

impl Eq for FileOverviewRank<'_> {}

impl PartialOrd for FileOverviewRank<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FileOverviewRank<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_file_overview_rank(self.0, other.0).reverse()
    }
}

/// Build a compact overview summary suitable for broad repo orientation.
pub fn compact_from_overview(
    overview: &GraphOverview,
    location_prefix: Option<&str>,
    hint: &str,
) -> GraphOverviewSummary {
    let mut dir_file_counts = BTreeMap::new();
    for file in &overview.files {
        let key = top_level_dir_key(&file.path, location_prefix);
        *dir_file_counts.entry(key).or_insert(0) += 1;
    }

    GraphOverviewSummary {
        total_dirs: overview.total_dirs,
        total_files: overview.total_files,
        total_symbols: overview.total_symbols,
        languages: overview.languages.clone(),
        symbol_kinds: overview.symbol_kinds.clone(),
        dir_file_counts,
        top_files: overview.top_files(10),
        hint: hint.to_string(),
    }
}

impl FileOverview {
    // pub(crate) widened for service/tests/overview.rs during test layout migration
    // (docs/design-patterns/test_layout.md, ORB-00249).
    pub(crate) fn top_file_entry(&self) -> TopFileEntry {
        TopFileEntry {
            selector: self.selector.clone(),
            name: self.name.clone(),
            symbol_count: self.symbol_count,
        }
    }
}

fn compare_file_overview_rank(left: &FileOverview, right: &FileOverview) -> Ordering {
    right
        .symbol_count
        .cmp(&left.symbol_count)
        .then_with(|| left.path.cmp(&right.path))
        .then_with(|| left.selector.cmp(&right.selector))
        .then_with(|| left.name.cmp(&right.name))
}

fn top_level_dir_key(path: &str, location_prefix: Option<&str>) -> String {
    let relative = location_prefix
        .and_then(|prefix| path.strip_prefix(prefix))
        .unwrap_or(path)
        .trim_start_matches('/');

    relative
        .split_once('/')
        .map(|(segment, _)| segment)
        .filter(|segment| !segment.is_empty())
        .unwrap_or(".")
        .to_string()
}
