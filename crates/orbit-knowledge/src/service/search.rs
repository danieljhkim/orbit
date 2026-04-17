use crate::graph::navigator::GraphNodeRef;

use super::{GraphContextService, SearchResult};

impl<'a> GraphContextService<'a> {
    pub fn search(
        &self,
        query: &str,
        node_types: Option<&[&str]>,
        location_prefix: Option<&str>,
        kind_filter: Option<&str>,
        limit: usize,
    ) -> Vec<GraphNodeRef<'a>> {
        if limit == 0 {
            return Vec::new();
        }

        let query_lower = query.to_lowercase();
        let browse = query_lower.is_empty();
        let mut results = Vec::new();

        for dir in &self.graph.dirs {
            let node = GraphNodeRef::Dir(dir);
            if self.node_matches(
                node,
                "dir",
                &query_lower,
                browse,
                node_types,
                location_prefix,
                kind_filter,
            ) {
                results.push(node);
                if results.len() >= limit {
                    return results;
                }
            }
        }
        for file in &self.graph.files {
            let node = GraphNodeRef::File(file);
            if self.node_matches(
                node,
                "file",
                &query_lower,
                browse,
                node_types,
                location_prefix,
                kind_filter,
            ) {
                results.push(node);
                if results.len() >= limit {
                    return results;
                }
            }
        }
        for leaf in &self.graph.leaves {
            let node = GraphNodeRef::Leaf(leaf);
            if self.node_matches(
                node,
                "symbol",
                &query_lower,
                browse,
                node_types,
                location_prefix,
                kind_filter,
            ) {
                results.push(node);
                if results.len() >= limit {
                    return results;
                }
            }
        }

        results
    }

    pub fn search_total(
        &self,
        query: &str,
        node_types: Option<&[&str]>,
        location_prefix: Option<&str>,
        kind_filter: Option<&str>,
    ) -> usize {
        let query_lower = query.to_lowercase();
        let browse = query_lower.is_empty();
        let mut total = 0usize;

        for dir in &self.graph.dirs {
            if self.node_matches(
                GraphNodeRef::Dir(dir),
                "dir",
                &query_lower,
                browse,
                node_types,
                location_prefix,
                kind_filter,
            ) {
                total += 1;
            }
        }
        for file in &self.graph.files {
            if self.node_matches(
                GraphNodeRef::File(file),
                "file",
                &query_lower,
                browse,
                node_types,
                location_prefix,
                kind_filter,
            ) {
                total += 1;
            }
        }
        for leaf in &self.graph.leaves {
            if self.node_matches(
                GraphNodeRef::Leaf(leaf),
                "symbol",
                &query_lower,
                browse,
                node_types,
                location_prefix,
                kind_filter,
            ) {
                total += 1;
            }
        }

        total
    }

    /// Search returning structured results with name, kind, and file info.
    pub fn search_structured(
        &self,
        query: &str,
        node_types: Option<&[&str]>,
        location_prefix: Option<&str>,
        kind_filter: Option<&str>,
        limit: usize,
    ) -> Vec<SearchResult> {
        let nodes = self.search(query, node_types, location_prefix, kind_filter, limit);
        nodes
            .into_iter()
            .map(|node| {
                let selector = self.selector_for_node(node);
                let name = node.base().name.to_string();
                let kind = match node {
                    GraphNodeRef::Dir(_) => "dir".to_string(),
                    GraphNodeRef::File(_) => "file".to_string(),
                    GraphNodeRef::Leaf(leaf) => leaf.kind.to_string(),
                };
                let file = match node {
                    GraphNodeRef::Leaf(leaf) => leaf
                        .base
                        .location
                        .split_once('#')
                        .map(|(path, _)| path.to_string()),
                    GraphNodeRef::File(file) => Some(file.base.location.clone()),
                    GraphNodeRef::Dir(_) => None,
                };
                SearchResult {
                    selector,
                    name,
                    kind,
                    file,
                }
            })
            .collect()
    }

    fn node_matches(
        &self,
        node: GraphNodeRef<'a>,
        node_type: &str,
        query_lower: &str,
        browse: bool,
        node_types: Option<&[&str]>,
        location_prefix: Option<&str>,
        kind_filter: Option<&str>,
    ) -> bool {
        if let Some(types) = node_types
            && !types.contains(&node_type)
        {
            return false;
        }
        if let Some(prefix) = location_prefix
            && !node.location().starts_with(prefix)
        {
            return false;
        }
        if let Some(kind_filter) = kind_filter {
            match node {
                GraphNodeRef::Leaf(leaf) if leaf.kind.to_string() == kind_filter => {}
                GraphNodeRef::Leaf(_) => return false,
                _ => return false,
            }
        }

        browse
            || node.base().name.to_lowercase().contains(query_lower)
            || node.location().to_lowercase().contains(query_lower)
    }
}
