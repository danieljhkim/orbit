#[path = "store/graph_io.rs"]
mod graph_io;
#[path = "store/leaf_data.rs"]
mod leaf_data;
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

pub use task_state::{
    load_task_working_graph, overlay_pack_with_working_graph, pack_from_working_graph,
    save_task_working_graph, task_working_graph_state_path,
};
pub use types::{KnowledgeEntryKind, KnowledgePack, KnowledgePackEntry, LeafData, SymbolSummary};

use graph_io::{GraphIndexFile, ManifestFile};

#[derive(Debug, Clone)]
pub struct KnowledgeStore {
    knowledge_dir: PathBuf,
    manifest: ManifestFile,
    graph_index: GraphIndexFile,
    selector_index: HashMap<SelectorLookupKey, String>,
}
