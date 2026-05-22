#[path = "store/graph_io.rs"]
mod graph_io;
#[path = "store/leaf_data.rs"]
mod leaf_data;
#[path = "store/object_cache.rs"]
mod object_cache;
#[path = "store/open.rs"]
mod open;
#[path = "store/pack.rs"]
mod pack;
#[path = "store/task_state.rs"]
mod task_state;
#[path = "store/types.rs"]
mod types;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::selector::SelectorLookupKey;

pub use object_cache::{
    DEFAULT_BLOB_CACHE_CAPACITY, DEFAULT_OBJECT_CACHE_CAPACITY, GraphObjectCache,
};
pub use task_state::{
    load_task_working_graph, overlay_pack_with_working_graph, pack_from_working_graph,
    save_task_working_graph, task_working_graph_state_path,
};
pub use types::{
    KnowledgeEntryKind, KnowledgePack, KnowledgePackAutoRefreshDiagnostic,
    KnowledgePackDiagnostics, KnowledgePackEntry, KnowledgePackErrorDiagnostic,
    KnowledgePackResult, KnowledgePackTimeout, LeafData, SymbolSummary, UnresolvedSelectorReason,
};

use graph_io::{GraphIndexFile, ManifestFile};

#[derive(Debug, Clone)]
pub struct KnowledgeStore {
    knowledge_dir: PathBuf,
    manifest: ManifestFile,
    graph_index: GraphIndexFile,
    selector_index: HashMap<SelectorLookupKey, String>,
    dir_children_index: HashMap<String, Vec<String>>,
    graph_object_cache: GraphObjectCache,
}

#[cfg(test)]
mod tests;
impl KnowledgeStore {
    pub fn graph_object_cache(&self) -> &GraphObjectCache {
        &self.graph_object_cache
    }
}
