//! Graph schema types, content-addressed persistence, and traversal.

pub(crate) mod call_extraction;
pub mod navigator;
pub mod nodes;
pub mod object_store;
pub(crate) mod source_match;
mod sqlite_index;

pub use navigator::{GraphNavigator, GraphNodeRef};
pub use nodes::{
    BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, GraphNode, LeafHistoryEntry, LeafKind,
    LeafNode, SignatureField,
};
pub use object_store::{GraphObjectStore, GraphReadOptions};
pub use sqlite_index::{
    GraphIndexCallerRow, GraphIndexNodeRow, GraphIndexReader, GraphIndexReferenceRow,
    GraphIndexSearchRow,
};

#[cfg(test)]
mod tests;
