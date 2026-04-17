use crate::error::KnowledgeError;
use crate::graph::navigator::GraphNodeRef;

use super::{GraphContextService, NodeContext, ReferenceHit};

impl<'a> GraphContextService<'a> {
    /// Build a selector string for a graph node.
    pub fn selector_for_node(&self, node: GraphNodeRef<'_>) -> String {
        match node {
            GraphNodeRef::Dir(dir) => {
                let path = dir.base.location.trim_end_matches('/');
                format!("dir:{path}")
            }
            GraphNodeRef::File(file) => format!("file:{}", file.base.location),
            GraphNodeRef::Leaf(leaf) => format!("symbol:{}:{}", leaf.base.location, leaf.kind),
        }
    }

    /// Get bounded context around a node: lineage, siblings, children.
    pub fn bounded_context(
        &self,
        node_id: &str,
        depth: usize,
        max_siblings: usize,
        max_children: usize,
    ) -> Result<NodeContext<'a>, KnowledgeError> {
        let node = self.nav.get_node(node_id)?;

        let lineage = self.nav.get_lineage(node_id, false)?;
        let bounded_lineage = if lineage.len() > depth {
            lineage[lineage.len() - depth..].to_vec()
        } else {
            lineage
        };

        let siblings = self.nav.get_siblings(node_id)?;
        let children = self.nav.get_children(node_id)?;

        Ok(NodeContext {
            node,
            lineage: bounded_lineage,
            siblings: siblings.into_iter().take(max_siblings).collect(),
            children: children.into_iter().take(max_children).collect(),
        })
    }

    /// Find graph nodes whose source mentions `symbol_name`.
    pub fn find_references(
        &self,
        symbol_names: &[String],
        definition_selector: Option<&str>,
    ) -> Vec<ReferenceHit> {
        if symbol_names.is_empty() {
            return Vec::new();
        }

        let mut hits = Vec::new();

        for leaf in &self.graph.leaves {
            let selector = self.selector_for_node(GraphNodeRef::Leaf(leaf));
            if definition_selector == Some(selector.as_str()) {
                continue;
            }

            let file_path = leaf
                .base
                .location
                .split_once('#')
                .map(|(path, _)| path)
                .unwrap_or(&leaf.base.location);

            if symbol_names
                .iter()
                .any(|symbol_name| source_mentions_symbol(&leaf.source, symbol_name))
            {
                hits.push(ReferenceHit {
                    selector,
                    name: leaf.base.name.clone(),
                    file: file_path.to_string(),
                    kind: leaf.kind.to_string(),
                });
            }
        }

        hits
    }
}

fn source_mentions_symbol(source: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }

    if !needle.chars().all(is_identifier_char) {
        return source.contains(needle);
    }

    let mut search_start = 0usize;
    while let Some(relative_match) = source[search_start..].find(needle) {
        let match_start = search_start + relative_match;
        let match_end = match_start + needle.len();
        let before = source[..match_start].chars().next_back();
        let after = source[match_end..].chars().next();
        let before_ok = before.is_none_or(|ch| !is_identifier_char(ch));
        let after_ok = after.is_none_or(|ch| !is_identifier_char(ch));
        if before_ok && after_ok {
            return true;
        }
        search_start = match_end;
    }

    false
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}
