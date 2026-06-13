use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::error::KnowledgeError;
use crate::graph::FileNode;
use crate::graph::navigator::GraphNodeRef;
use crate::graph::source_match::source_mentions_symbol;

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

    /// Find graph nodes whose source or containing file mentions one of the
    /// requested symbol names.
    pub fn find_references(
        &self,
        knowledge_dir: Option<&Path>,
        symbol_names: &[String],
        definition_selector: Option<&str>,
    ) -> Vec<ReferenceHit> {
        if symbol_names.is_empty() {
            return Vec::new();
        }

        let mut hits = Vec::new();
        let mut seen_selectors = HashSet::new();
        let mut leaf_hit_files = HashSet::new();
        let definition_file_selector =
            definition_selector.and_then(file_selector_for_definition_selector);

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
                && seen_selectors.insert(selector.clone())
            {
                leaf_hit_files.insert(file_path.to_string());
                hits.push(ReferenceHit {
                    selector,
                    name: leaf.base.name.clone(),
                    file: file_path.to_string(),
                    kind: leaf.kind.to_string(),
                });
            }
        }

        for file in &self.graph.files {
            let selector = self.selector_for_node(GraphNodeRef::File(file));
            if definition_file_selector.as_deref() == Some(selector.as_str())
                || leaf_hit_files.contains(&file.base.location)
                || !seen_selectors.insert(selector.clone())
            {
                continue;
            }

            if file_mentions_symbol(knowledge_dir, file, symbol_names) {
                hits.push(ReferenceHit {
                    selector,
                    name: file.base.name.clone(),
                    file: file.base.location.clone(),
                    kind: "file".to_string(),
                });
            }
        }

        hits
    }
}

fn file_selector_for_definition_selector(selector: &str) -> Option<String> {
    selector
        .strip_prefix("symbol:")
        .and_then(|rest| rest.split_once('#').map(|(path, _)| format!("file:{path}")))
}

fn file_mentions_symbol(
    knowledge_dir: Option<&Path>,
    file: &FileNode,
    symbol_names: &[String],
) -> bool {
    if file.imports.iter().any(|import| {
        symbol_names
            .iter()
            .any(|needle| source_mentions_symbol(import, needle))
    }) {
        return true;
    }

    let Some(knowledge_dir) = knowledge_dir else {
        return false;
    };
    let Some(blob_hash) = file.source_blob_hash.as_deref() else {
        return false;
    };
    if blob_hash.len() < 2 {
        return false;
    }

    let blob_path = knowledge_dir
        .join("graph/blobs")
        .join(&blob_hash[..2])
        .join(format!("{blob_hash}.txt"));
    let Ok(source) = fs::read_to_string(blob_path) else {
        return false;
    };

    symbol_names
        .iter()
        .any(|needle| source_mentions_symbol(&source, needle))
}
