use std::collections::HashMap;

use crate::error::KnowledgeError;

use super::nodes::{BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafNode};

/// Borrowed reference to a graph node.
#[derive(Debug, Clone, Copy)]
pub enum GraphNodeRef<'a> {
    Dir(&'a DirNode),
    File(&'a FileNode),
    Leaf(&'a LeafNode),
}

impl<'a> GraphNodeRef<'a> {
    pub fn id(&self) -> &str {
        &self.base().id
    }

    pub fn base(&self) -> &BaseNodeFields {
        match self {
            GraphNodeRef::Dir(n) => &n.base,
            GraphNodeRef::File(n) => &n.base,
            GraphNodeRef::Leaf(n) => &n.base,
        }
    }

    pub fn parent_id(&self) -> Option<&str> {
        self.base().parent_id.as_deref()
    }

    pub fn location(&self) -> &str {
        &self.base().location
    }

    pub fn child_ids(&self) -> Vec<&str> {
        match self {
            GraphNodeRef::Dir(n) => n
                .dir_children
                .iter()
                .chain(n.file_children.iter())
                .map(String::as_str)
                .collect(),
            GraphNodeRef::File(n) => n.leaf_children.iter().map(String::as_str).collect(),
            GraphNodeRef::Leaf(n) => n.children.iter().map(String::as_str).collect(),
        }
    }
}

/// Provides traversal methods over a [`CodebaseGraphV1`].
///
/// Builds an internal index on construction for O(1) lookups by node ID.
pub struct GraphNavigator<'a> {
    graph: &'a CodebaseGraphV1,
    node_index: HashMap<&'a str, GraphNodeRef<'a>>,
}

impl<'a> GraphNavigator<'a> {
    pub fn new(graph: &'a CodebaseGraphV1) -> Self {
        let mut node_index = HashMap::new();
        for dir in &graph.dirs {
            node_index.insert(dir.base.id.as_str(), GraphNodeRef::Dir(dir));
        }
        for file in &graph.files {
            node_index.insert(file.base.id.as_str(), GraphNodeRef::File(file));
        }
        for leaf in &graph.leaves {
            node_index.insert(leaf.base.id.as_str(), GraphNodeRef::Leaf(leaf));
        }
        Self { graph, node_index }
    }

    pub fn get_node(&self, id: &str) -> Result<GraphNodeRef<'a>, KnowledgeError> {
        self.node_index
            .get(id)
            .copied()
            .ok_or_else(|| KnowledgeError::invalid_data(format!("node not found: {id}")))
    }

    pub fn get_root(&self) -> Result<GraphNodeRef<'a>, KnowledgeError> {
        self.get_node(&self.graph.root_dir_id)
    }

    pub fn get_parent(&self, id: &str) -> Result<Option<GraphNodeRef<'a>>, KnowledgeError> {
        let node = self.get_node(id)?;
        match node.parent_id() {
            Some(pid) => Ok(Some(self.get_node(pid)?)),
            None => Ok(None),
        }
    }

    pub fn get_children(&self, id: &str) -> Result<Vec<GraphNodeRef<'a>>, KnowledgeError> {
        let node = self.get_node(id)?;
        node.child_ids()
            .into_iter()
            .map(|cid| self.get_node(cid))
            .collect()
    }

    pub fn get_siblings(&self, id: &str) -> Result<Vec<GraphNodeRef<'a>>, KnowledgeError> {
        let node = self.get_node(id)?;
        let parent_id = match node.parent_id() {
            Some(pid) => pid,
            None => return Ok(vec![]),
        };
        let parent = self.get_node(parent_id)?;
        parent
            .child_ids()
            .into_iter()
            .filter(|cid| *cid != id)
            .map(|cid| self.get_node(cid))
            .collect()
    }

    /// Returns the lineage (ancestor chain) for a node, in root-first order.
    pub fn get_lineage(
        &self,
        id: &str,
        include_self: bool,
    ) -> Result<Vec<GraphNodeRef<'a>>, KnowledgeError> {
        let node = self.get_node(id)?;
        let mut chain = Vec::new();
        let mut current = node;
        while let Some(pid) = current.parent_id() {
            let parent = self.get_node(pid)?;
            chain.push(parent);
            current = parent;
        }
        chain.reverse();
        if include_self {
            chain.push(node);
        }
        Ok(chain)
    }

    /// For a leaf, walk up ancestors to find the containing file node.
    pub fn get_containing_file(&self, id: &str) -> Result<Option<&'a FileNode>, KnowledgeError> {
        let mut current = self.get_node(id)?;
        loop {
            match current {
                GraphNodeRef::File(f) => return Ok(Some(f)),
                _ => match current.parent_id() {
                    Some(pid) => current = self.get_node(pid)?,
                    None => return Ok(None),
                },
            }
        }
    }
}
