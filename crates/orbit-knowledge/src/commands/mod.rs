//! Canonical command surface for knowledge-graph operations.
//!
//! Tool adapters should parse request envelopes, call these commands, and
//! shape the returned typed results for their transport.

use std::path::PathBuf;

use orbit_common::types::OrbitError;

use crate::graph::object_store::{GraphObjectStore, resolve_graph_read_target};
use crate::graph::{GraphIndexReader, GraphReadOptions};
use crate::service::TaskGraphService;
use crate::{KnowledgeError, KnowledgeErrorKind};

pub mod callers;
pub mod deps;
pub mod implementors;
pub mod overview;
pub mod pack;
pub mod refs;
pub mod search;
pub mod show;
pub mod write;

pub use crate::service::{TaskGraphScope, default_knowledge_dir};

#[derive(Debug, Clone)]
pub struct GraphCommandContext {
    pub knowledge_dir: PathBuf,
    pub workspace_root: Option<PathBuf>,
    pub explicit_ref: Option<String>,
    pub explicit_knowledge_dir: bool,
    pub task_scope: TaskGraphScope,
}

impl GraphCommandContext {
    pub(crate) fn read_graph(
        &self,
        options: GraphReadOptions,
    ) -> Result<crate::graph::nodes::CodebaseGraphV1, KnowledgeError> {
        let service = TaskGraphService::new(self.knowledge_dir.clone(), self.task_scope.clone());
        service
            .read_graph(
                self.workspace_root.as_deref(),
                self.explicit_knowledge_dir,
                self.explicit_ref.as_deref(),
                options,
            )
            .map_err(knowledge_error_from_orbit)
    }

    pub(crate) fn task_service(&self) -> TaskGraphService {
        TaskGraphService::new(self.knowledge_dir.clone(), self.task_scope.clone())
    }

    pub(crate) fn open_current_graph_index(
        &self,
    ) -> Result<Option<GraphIndexReader>, KnowledgeError> {
        if self.explicit_ref.is_none()
            && !self.explicit_knowledge_dir
            && let Some(workspace_root) = self.workspace_root.as_deref()
        {
            let _ = crate::pipeline::ensure_fresh(&self.knowledge_dir, workspace_root);
        }

        let read_target = match resolve_graph_read_target(
            self.workspace_root.as_deref(),
            self.explicit_ref.as_deref(),
        ) {
            Ok(target) => target,
            Err(_) => return Ok(None),
        };
        let graph_store = GraphObjectStore::new(self.knowledge_dir.join("graph"));
        if graph_store
            .prepare_refs_layout(read_target.default.as_ref())
            .is_err()
        {
            return Ok(None);
        }
        let resolved =
            match graph_store.resolve_ref(&read_target.requested, read_target.fallback.as_ref()) {
                Ok(resolved) => resolved,
                Err(_) => return Ok(None),
            };

        GraphIndexReader::open_current(
            graph_store.graph_sqlite_index_path(),
            &resolved.current_ref.root_graph_hash,
        )
        .map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!("open graph sqlite index: {error}"))
        })
    }
}

pub(crate) fn knowledge_error_from_orbit(error: OrbitError) -> KnowledgeError {
    match error {
        OrbitError::InvalidInput(message) => KnowledgeError::invalid_data(message),
        OrbitError::Execution(message) => KnowledgeError::knowledge_unavailable(message),
        other => KnowledgeError::knowledge_unavailable(other.to_string()),
    }
}

/// Translate a [`KnowledgeError`] into an [`OrbitError`] for crates that
/// expose a workspace-wide error surface (orbit-core, orbit-tools). The
/// `knowledge_invalid` kind maps to `InvalidInput` because callers treat it
/// as user-input error; every other kind maps to `Execution`.
pub fn knowledge_error_to_orbit(error: KnowledgeError) -> OrbitError {
    let KnowledgeError { kind, reason } = error;
    match kind {
        KnowledgeErrorKind::Invalid => OrbitError::InvalidInput(reason),
        KnowledgeErrorKind::Unavailable | KnowledgeErrorKind::Io => {
            OrbitError::Execution(format!("{kind}: {reason}"))
        }
    }
}
