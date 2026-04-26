use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use serde_json::Value;

use crate::extract::{self, Language};
use crate::graph::object_store::{GraphObjectStore, resolve_graph_read_target};
use crate::lock::GraphLockGuard;
use crate::pipeline::context::BuildConfig;
use crate::{
    KnowledgeError, KnowledgeStore, Selector, WorkingGraph, WorkingLeaf, load_task_working_graph,
    overlay_pack_with_working_graph, pack_from_working_graph, save_task_working_graph,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskGraphScope {
    pub orbit_root: Option<PathBuf>,
    pub task_id: Option<String>,
    pub owner: String,
}

#[derive(Debug, Clone)]
pub struct TaskGraphService {
    knowledge_dir: PathBuf,
    scope: TaskGraphScope,
}

pub fn default_knowledge_dir(workspace_root: &Path, orbit_root: Option<&Path>) -> PathBuf {
    orbit_root
        .map(|root| root.join("knowledge"))
        .unwrap_or_else(|| workspace_root.join(".orbit/knowledge"))
}

impl TaskGraphService {
    pub fn new(knowledge_dir: PathBuf, scope: TaskGraphScope) -> Self {
        Self {
            knowledge_dir,
            scope,
        }
    }

    pub fn pack_json(
        &self,
        selectors: &[Selector],
        workspace_root: Option<&Path>,
        explicit_knowledge_dir: bool,
        explicit_ref: Option<&str>,
    ) -> Result<Value, OrbitError> {
        if explicit_ref.is_none() {
            self.maybe_refresh_knowledge_graph(workspace_root, explicit_knowledge_dir);
        }

        let read_target = resolve_graph_read_target(workspace_root, explicit_ref)?;
        let working_graph = if explicit_ref.is_some() {
            None
        } else {
            load_task_working_graph(
                self.scope.orbit_root.as_deref(),
                self.scope.task_id.as_deref(),
            )?
        };

        let pack_result = || -> Result<_, KnowledgeError> {
            let store = KnowledgeStore::open(
                &self.knowledge_dir,
                &read_target.requested,
                read_target.fallback.as_ref(),
                read_target.default.as_ref(),
            )?;
            store.pack(selectors)
        };

        let pack = match pack_result() {
            Ok(pack) => pack,
            Err(first_error) => {
                let pack_or_error = if explicit_ref.is_some() {
                    Err(first_error)
                } else {
                    match self.rebuild_default_knowledge_graph(
                        workspace_root,
                        explicit_knowledge_dir,
                        &first_error,
                    ) {
                        Ok(true) => match pack_result() {
                            Ok(pack) => Ok(pack),
                            Err(retry_error) => Err(KnowledgeError {
                                kind: "knowledge_unavailable".to_string(),
                                reason: format!(
                                    "failed to load knowledge pack: {first_error}; retry after rebuild failed: {retry_error}"
                                ),
                            }),
                        },
                        Ok(false) => Err(first_error),
                        Err(rebuild_error) => Err(KnowledgeError {
                            kind: "knowledge_unavailable".to_string(),
                            reason: format!(
                                "failed to load knowledge pack: {first_error}; rebuild attempt failed: {rebuild_error}"
                            ),
                        }),
                    }
                };

                match pack_or_error {
                    Ok(pack) => pack,
                    Err(error) => {
                        if let Some(graph) = working_graph.as_ref() {
                            let pack =
                                pack_from_working_graph(&self.knowledge_dir, selectors, graph);
                            return serde_json::to_value(pack).map_err(|serialize| {
                                OrbitError::Execution(format!(
                                    "failed to serialize knowledge pack: {serialize}"
                                ))
                            });
                        }
                        return serde_json::to_value(error).map_err(|serialize| {
                            OrbitError::Execution(format!(
                                "failed to serialize knowledge error: {serialize}"
                            ))
                        });
                    }
                }
            }
        };

        let pack = if let Some(graph) = working_graph.as_ref() {
            overlay_pack_with_working_graph(pack, selectors, graph)
        } else {
            pack
        };

        serde_json::to_value(pack)
            .map_err(|error| OrbitError::Execution(format!("serialize knowledge pack: {error}")))
    }

    pub fn mutate<T, F>(
        &self,
        selector: &Selector,
        extra_files: &[&str],
        reason: &str,
        workspace_root: &Path,
        op: F,
    ) -> Result<T, OrbitError>
    where
        F: FnOnce(&mut WorkingGraph) -> Result<T, OrbitError>,
    {
        let mut working_graph = match load_task_working_graph(
            self.scope.orbit_root.as_deref(),
            self.scope.task_id.as_deref(),
        )? {
            Some(graph) => graph,
            None => initialize_working_graph(&self.knowledge_dir, selector, workspace_root)?,
        };

        let lock_targets = lock_targets_for_mutation(selector, extra_files);
        let mut guard = GraphLockGuard::acquire(
            &self.knowledge_dir,
            &self.scope.owner,
            self.scope.task_id.as_deref(),
            reason,
            &lock_targets,
        )
        .map_err(|error| OrbitError::Execution(format!("acquire graph locks: {error}")))?;

        let operation_result = op(&mut working_graph).and_then(|value| {
            save_task_working_graph(
                self.scope.orbit_root.as_deref(),
                self.scope.task_id.as_deref(),
                &working_graph,
            )?;
            Ok(value)
        });
        let unlock_result = guard
            .release()
            .map_err(|error| OrbitError::Execution(format!("release graph locks: {error}")));

        match (operation_result, unlock_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(unlock_error)) => Err(unlock_error),
            (Err(error), Err(unlock_error)) => Err(OrbitError::Execution(format!(
                "{error}; also failed to release graph locks: {unlock_error}"
            ))),
        }
    }

    pub fn read_graph(
        &self,
        workspace_root: Option<&Path>,
        explicit_knowledge_dir: bool,
        explicit_ref: Option<&str>,
    ) -> Result<crate::graph::nodes::CodebaseGraphV1, OrbitError> {
        if explicit_ref.is_none() {
            self.maybe_refresh_knowledge_graph(workspace_root, explicit_knowledge_dir);
        }
        let read_target = resolve_graph_read_target(workspace_root, explicit_ref)?;

        let graph_dir = self.knowledge_dir.join("graph");
        match GraphObjectStore::new(&graph_dir).read_graph(
            &read_target.requested,
            read_target.fallback.as_ref(),
            read_target.default.as_ref(),
        ) {
            Ok(graph) => Ok(graph),
            Err(first_error) => {
                if explicit_ref.is_some() {
                    return Err(OrbitError::Execution(format!(
                        "failed to load knowledge graph: {first_error}"
                    )));
                }
                let rebuilt = self
                    .rebuild_default_knowledge_graph(
                        workspace_root,
                        explicit_knowledge_dir,
                        &first_error,
                    )
                    .map_err(|rebuild_error| {
                        OrbitError::Execution(format!(
                            "failed to load knowledge graph: {first_error}; rebuild attempt failed: {rebuild_error}"
                        ))
                    })?;
                if !rebuilt {
                    return Err(OrbitError::Execution(format!(
                        "failed to load knowledge graph: {first_error}"
                    )));
                }

                GraphObjectStore::new(&graph_dir)
                    .read_graph(
                        &read_target.requested,
                        read_target.fallback.as_ref(),
                        read_target.default.as_ref(),
                    )
                    .map_err(|retry_error| {
                        OrbitError::Execution(format!(
                            "failed to load knowledge graph: {first_error}; retry after rebuild failed: {retry_error}"
                        ))
                    })
            }
        }
    }

    fn maybe_refresh_knowledge_graph(
        &self,
        workspace_root: Option<&Path>,
        explicit_knowledge_dir: bool,
    ) {
        if explicit_knowledge_dir {
            return;
        }

        let Some(workspace_root) = workspace_root else {
            return;
        };

        if let Err(error) = crate::pipeline::ensure_fresh(&self.knowledge_dir, workspace_root) {
            eprintln!("warning: knowledge graph auto-refresh failed: {error}");
        }
    }

    fn rebuild_default_knowledge_graph(
        &self,
        workspace_root: Option<&Path>,
        explicit_knowledge_dir: bool,
        first_error: &KnowledgeError,
    ) -> Result<bool, String> {
        if explicit_knowledge_dir {
            return Ok(false);
        }

        let Some(workspace_root) = workspace_root else {
            return Ok(false);
        };

        eprintln!(
            "warning: knowledge graph load failed: {first_error}; rebuilding default knowledge graph at {}",
            self.knowledge_dir.display()
        );
        let incremental = self.knowledge_dir.join("manifest.json").is_file();
        crate::pipeline::run_build(BuildConfig {
            repo_path: workspace_root.to_path_buf(),
            output_dir: self.knowledge_dir.clone(),
            incremental,
            ref_name: None,
            task_id_pattern: None,
        })
        .map_err(|error| error.to_string())?;
        Ok(true)
    }
}

fn lock_targets_for_mutation(selector: &Selector, extra_files: &[&str]) -> Vec<String> {
    let mut targets = Vec::new();
    match selector {
        Selector::File { path } => targets.push(format!("file:{path}")),
        Selector::Symbol { path, .. } => {
            targets.push(selector.to_string());
            targets.push(format!("file:{path}"));
        }
        Selector::Dir { .. } => {}
    }

    for file in extra_files {
        let selector = format!("file:{file}");
        if !targets.contains(&selector) {
            targets.push(selector);
        }
    }

    targets
}

fn initialize_working_graph(
    knowledge_dir: &Path,
    selector: &Selector,
    workspace_root: &Path,
) -> Result<WorkingGraph, OrbitError> {
    if let Ok(read_target) = resolve_graph_read_target(Some(workspace_root), None)
        && let Ok(store) = KnowledgeStore::open(
            knowledge_dir,
            &read_target.requested,
            read_target.fallback.as_ref(),
            read_target.default.as_ref(),
        )
        && let Ok(mut graph) = WorkingGraph::from_store(&store)
    {
        graph.seed_file_snapshots_from_workspace(workspace_root);
        return Ok(graph);
    }

    let Selector::Symbol { path, .. } = selector else {
        return Ok(WorkingGraph::new());
    };

    let file_path = workspace_root.join(path);
    let ext = file_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let Some(language) = Language::from_extension(ext) else {
        return Ok(WorkingGraph::new());
    };

    let content = std::fs::read_to_string(&file_path)
        .map_err(|error| OrbitError::Execution(format!("read {}: {error}", file_path.display())))?;

    let extraction = extract::extract_file(&content, language);
    let mut graph = WorkingGraph::new();

    for leaf in &extraction.leaves {
        let selector = format!("symbol:{path}#{}:{}", leaf.qualified_name, leaf.kind);
        graph.insert_working_leaf(
            selector.clone(),
            WorkingLeaf {
                selector,
                file_path: path.clone(),
                name: leaf.name.clone(),
                qualified_name: leaf.qualified_name.clone(),
                kind: leaf.kind.clone(),
                start_line: leaf.start_line,
                end_line: leaf.end_line,
                source: leaf.source.clone(),
                source_hash: leaf.source_hash.clone(),
                parent_qualified_name: leaf.parent_qualified_name.clone(),
                children_qualified_names: leaf.children_qualified_names.clone(),
            },
        );
    }
    graph.seed_file_snapshots_from_workspace(workspace_root);

    Ok(graph)
}
