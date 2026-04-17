mod error;
pub mod extract;
pub mod graph;
mod io;
pub mod lock;
pub mod pipeline;
mod selector;
pub mod service;
mod store;
pub mod working_graph;

pub use error::KnowledgeError;
pub use selector::{Selector, SelectorParseError};
pub use service::{TaskGraphScope, TaskGraphService, default_knowledge_dir};
pub use store::{
    KnowledgeEntryKind, KnowledgePack, KnowledgePackEntry, KnowledgeStore, LeafData, SymbolSummary,
    load_task_working_graph, overlay_pack_with_working_graph, pack_from_working_graph,
    save_task_working_graph, task_working_graph_state_path,
};
pub use working_graph::{
    LeafEdit, LeafVersionChain, MoveResult, WorkingGraph, WorkingLeaf, WriteError, WriteResult,
};
