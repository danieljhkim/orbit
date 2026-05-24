use std::collections::HashMap;

use crate::error::KnowledgeError;
use crate::graph::navigator::GraphNodeRef;
use orbit_graph_extract::Selector;

use super::GraphContextService;

impl<'a> GraphContextService<'a> {
    pub fn new(graph: &'a crate::graph::nodes::CodebaseGraphV1) -> Self {
        let nav = crate::graph::navigator::GraphNavigator::new(graph);
        let mut location_index = HashMap::new();

        for dir in &graph.dirs {
            let key = dir.base.location.trim_end_matches('/').to_string();
            location_index.insert(key, dir.base.id.as_str());
        }
        for file in &graph.files {
            location_index.insert(file.base.location.clone(), file.base.id.as_str());
        }
        for leaf in &graph.leaves {
            let key = format!("{}:{}", leaf.base.location, leaf.kind);
            location_index.insert(key, leaf.base.id.as_str());
        }

        Self {
            graph,
            nav,
            location_index,
        }
    }

    pub fn navigator(&self) -> &crate::graph::navigator::GraphNavigator<'a> {
        &self.nav
    }

    /// Resolve a [`Selector`] to a graph node.
    pub fn resolve_selector(
        &self,
        selector: &Selector,
    ) -> Result<GraphNodeRef<'a>, KnowledgeError> {
        let key = match selector {
            Selector::Dir { path } => path.trim_end_matches('/').to_string(),
            Selector::File { path } => path.clone(),
            Selector::Symbol { path, symbol, kind } => format!("{path}#{symbol}:{kind}"),
        };

        let node_id = self.location_index.get(key.as_str()).ok_or_else(|| {
            KnowledgeError::invalid_data(format!(
                "selector `{selector}` does not resolve to a node"
            ))
        })?;

        self.nav.get_node(node_id)
    }

    /// Resolve multiple selectors, returning (resolved, unresolved) pairs.
    pub fn resolve_many(&self, selectors: &[Selector]) -> (Vec<GraphNodeRef<'a>>, Vec<String>) {
        let mut resolved = Vec::new();
        let mut unresolved = Vec::new();
        for selector in selectors {
            match self.resolve_selector(selector) {
                Ok(node) => resolved.push(node),
                Err(_) => unresolved.push(selector.to_string()),
            }
        }
        (resolved, unresolved)
    }
}
