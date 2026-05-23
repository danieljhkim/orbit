//! Graph schema types, content-addressed persistence, and traversal.

pub mod navigator;
pub mod nodes;
pub mod object_store;
mod sqlite_index;

pub use navigator::{GraphNavigator, GraphNodeRef};
pub use nodes::{
    BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, GraphNode, LeafHistoryEntry, LeafKind,
    LeafNode, SignatureField,
};
pub use object_store::{GraphObjectStore, GraphReadOptions};
pub use sqlite_index::{GraphIndexNodeRow, GraphIndexReader, GraphIndexSearchRow};

#[cfg(test)]
mod tests;
