use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use orbit_knowledge::KnowledgeError;
use orbit_knowledge::commands::refs::{self, RefInclude, RefsInput};
use orbit_knowledge::commands::search::{self, SearchInput};
use orbit_knowledge::commands::show::{self, ShowInput, ShowNodeDetails};
use orbit_knowledge::commands::{GraphCommandContext, TaskGraphScope, default_knowledge_dir};

pub(crate) type BackendResult<T> = Result<T, BackendError>;
pub(crate) type SearchOutput = Vec<SearchEntry>;
pub(crate) type ShowOutput = Option<String>;
pub(crate) type RefsOutput = Vec<(String, Option<u32>, String)>;
pub(crate) type CalleesOutput = Vec<(String, Option<u32>, String)>;
pub(crate) type ImpactOutput = Vec<String>;

pub(crate) trait Backend {
    fn search(&self, query: &str) -> BackendResult<SearchOutput>;
    fn show(&self, selector: &str) -> BackendResult<ShowOutput>;
    fn refs(&self, selector: &str) -> BackendResult<RefsOutput>;
    fn callees(&self, selector: &str) -> BackendResult<CalleesOutput>;
    fn impact(&self, selector: &str, depth: u8) -> BackendResult<ImpactOutput>;
}

#[derive(Debug, Clone)]
pub(crate) struct V1Backend {
    context: GraphCommandContext,
    limit: usize,
}

impl V1Backend {
    pub(crate) fn for_workspace(workspace_root: PathBuf, knowledge_dir: Option<PathBuf>) -> Self {
        let knowledge_dir =
            knowledge_dir.unwrap_or_else(|| default_knowledge_dir(&workspace_root, None));
        Self::new(GraphCommandContext {
            knowledge_dir,
            workspace_root: Some(workspace_root),
            explicit_ref: None,
            explicit_knowledge_dir: false,
            task_scope: TaskGraphScope::default(),
        })
    }

    pub(crate) fn new(context: GraphCommandContext) -> Self {
        Self { context, limit: 20 }
    }
}

impl Backend for V1Backend {
    fn search(&self, query: &str) -> BackendResult<SearchOutput> {
        let result = search::run(SearchInput {
            context: self.context.clone(),
            query: query.to_string(),
            node_type: None,
            kind_filter: None,
            prefix: None,
            source_regex: None,
            include_non_code: false,
            allow_fuzzy: false,
            limit: self.limit,
        })?;

        Ok(result
            .hits
            .into_iter()
            .map(|hit| SearchEntry {
                selector: hit.selector,
                kind: hit.kind,
                file: hit.file,
                name: hit.name,
            })
            .collect())
    }

    fn show(&self, selector: &str) -> BackendResult<ShowOutput> {
        let result = show::run(ShowInput {
            context: self.context.clone(),
            selector: selector.to_string(),
            depth: 0,
            max_siblings: 0,
            max_children: 0,
        })?;

        let source = match result.details {
            ShowNodeDetails::Leaf { source, .. } => Some(source),
            ShowNodeDetails::File { source, .. } => source,
            ShowNodeDetails::Dir => None,
        };
        Ok(source)
    }

    fn refs(&self, selector: &str) -> BackendResult<RefsOutput> {
        let result = refs::run(RefsInput {
            context: self.context.clone(),
            selector: selector.to_string(),
            include_simple_name: true,
            include: RefInclude::code_only(),
            limit: self.limit,
            per_file_limit: 5,
        })?;

        Ok(result
            .code_refs
            .into_iter()
            .map(|hit| (hit.file, None, hit.kind))
            .collect())
    }

    fn callees(&self, _selector: &str) -> BackendResult<CalleesOutput> {
        Err(BackendError::Unsupported(
            "orbit-knowledge does not expose a callees command yet",
        ))
    }

    fn impact(&self, _selector: &str, _depth: u8) -> BackendResult<ImpactOutput> {
        Err(BackendError::Unsupported(
            "orbit-knowledge does not expose an impact command yet",
        ))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct V2Backend;

impl Backend for V2Backend {
    fn search(&self, _query: &str) -> BackendResult<SearchOutput> {
        unimplemented!("orbit-graph not yet wired")
    }

    fn show(&self, _selector: &str) -> BackendResult<ShowOutput> {
        unimplemented!("orbit-graph not yet wired")
    }

    fn refs(&self, _selector: &str) -> BackendResult<RefsOutput> {
        unimplemented!("orbit-graph not yet wired")
    }

    fn callees(&self, _selector: &str) -> BackendResult<CalleesOutput> {
        unimplemented!("orbit-graph not yet wired")
    }

    fn impact(&self, _selector: &str, _depth: u8) -> BackendResult<ImpactOutput> {
        unimplemented!("orbit-graph not yet wired")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchEntry {
    pub(crate) selector: String,
    pub(crate) kind: String,
    pub(crate) file: Option<String>,
    pub(crate) name: String,
}

#[derive(Debug)]
pub(crate) enum BackendError {
    Knowledge(KnowledgeError),
    Unsupported(&'static str),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Knowledge(error) => write!(f, "{error}"),
            Self::Unsupported(message) => f.write_str(message),
        }
    }
}

impl Error for BackendError {}

impl From<KnowledgeError> for BackendError {
    fn from(error: KnowledgeError) -> Self {
        Self::Knowledge(error)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use orbit_knowledge::commands::{GraphCommandContext, TaskGraphScope};
    use orbit_knowledge::graph::object_store::{GraphObjectStore, RefName};
    use orbit_knowledge::graph::{
        BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode,
    };

    use super::{Backend, V1Backend};

    #[test]
    fn v1_backend_returns_real_search_and_show_results() -> Result<(), Box<dyn std::error::Error>> {
        let temp_path = unique_temp_path()?;
        fs::create_dir_all(&temp_path)?;

        let result = run_v1_smoke(&temp_path);
        let cleanup_result = fs::remove_dir_all(&temp_path);

        result?;
        cleanup_result?;
        Ok(())
    }

    fn run_v1_smoke(temp_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let store = GraphObjectStore::new(temp_path.join("graph"));
        let current_ref = store.write_graph(&smoke_graph())?;
        let ref_name = RefName::new("graph-equiv-smoke")?;
        store.write_ref_atomic(&ref_name, &current_ref)?;

        let backend = V1Backend::new(GraphCommandContext {
            knowledge_dir: temp_path.to_path_buf(),
            workspace_root: None,
            explicit_ref: Some(ref_name.as_str().to_string()),
            explicit_knowledge_dir: true,
            task_scope: TaskGraphScope::default(),
        });

        let search = backend.search("fixture_fn")?;
        assert!(search.iter().any(|hit| hit.name == "fixture_fn"));

        let show = backend.show("symbol:src/fixture.rs#fixture_fn:function")?;
        assert_eq!(show.as_deref(), Some("fn fixture_fn() {}\n"));

        Ok(())
    }

    fn unique_temp_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        Ok(std::env::temp_dir().join(format!("graph-equiv-{}-{nanos}", std::process::id())))
    }

    fn smoke_graph() -> CodebaseGraphV1 {
        let root_id = "dir:.".to_string();
        let file_id = "file:src/fixture.rs".to_string();
        let leaf_id = "symbol:src/fixture.rs#fixture_fn:function".to_string();

        CodebaseGraphV1 {
            root_dir_id: root_id.clone(),
            dirs: vec![DirNode {
                base: base_node(&root_id, ".", ".", "", None),
                dir_children: Vec::new(),
                file_children: vec![file_id.clone()],
            }],
            files: vec![FileNode {
                base: base_node(
                    &file_id,
                    "fixture.rs",
                    "src/fixture.rs",
                    "rust",
                    Some(root_id),
                ),
                extension: Some("rs".to_string()),
                source_blob_hash: None,
                source: "fn fixture_fn() {}\n".to_string(),
                imports: Vec::new(),
                exports: Vec::new(),
                re_exports: Vec::new(),
                leaf_children: vec![leaf_id.clone()],
            }],
            leaves: vec![LeafNode {
                base: base_node(
                    &leaf_id,
                    "fixture_fn",
                    "src/fixture.rs#fixture_fn",
                    "rust",
                    Some(file_id),
                ),
                kind: LeafKind::Function,
                source: "fn fixture_fn() {}\n".to_string(),
                source_blob_hash: None,
                source_hash: None,
                file_hash_at_capture: None,
                history: Vec::new(),
                input_signature: Vec::new(),
                output_signature: Vec::new(),
                start_line: Some(1),
                end_line: Some(1),
                children: Vec::new(),
            }],
        }
    }

    fn base_node(
        id: &str,
        name: &str,
        location: &str,
        language: &str,
        parent_id: Option<String>,
    ) -> BaseNodeFields {
        BaseNodeFields {
            id: id.to_string(),
            identity_key: id.to_string(),
            object_hash: None,
            name: name.to_string(),
            location: location.to_string(),
            language: language.to_string(),
            description: String::new(),
            parent_id,
            is_locked: false,
            lineage_locked: false,
            lock_owner: None,
            lock_reason: String::new(),
        }
    }
}
