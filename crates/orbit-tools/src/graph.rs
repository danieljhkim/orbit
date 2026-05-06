use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use orbit_knowledge::graph::navigator::GraphNodeRef;
use orbit_knowledge::graph::nodes::CodebaseGraphV1;
use orbit_knowledge::graph::object_store::RefName;
use orbit_knowledge::pipeline::context::BuildConfig;
use orbit_knowledge::service::GraphContextService;
use orbit_knowledge::{Selector, TaskGraphScope, TaskGraphService};
use serde_json::{Value, json};

pub(crate) const REMOVED_GRAPH_HISTORY_MESSAGE: &str = "Knowledge-graph task attribution has been removed. Use `git log --grep '[T<task-id>]'` for local forward lookup, and use `external_refs` for cross-engineer task references.";

#[derive(Debug, Clone)]
pub struct GraphBuildOptions {
    pub data_root: PathBuf,
    pub repo_override: Option<PathBuf>,
    pub ref_name: Option<String>,
    pub incremental: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedGraphBuild {
    pub mode: &'static str,
    pub repo_path: PathBuf,
    pub output_dir: PathBuf,
    incremental: bool,
    ref_name: Option<RefName>,
}

#[derive(Debug, Clone)]
pub struct GraphBuildOutput {
    pub mode: &'static str,
    pub output_dir: PathBuf,
    pub dirs: usize,
    pub files: usize,
    pub leaves: usize,
}

#[derive(Debug, Clone)]
pub struct GraphShowOptions {
    pub data_root: PathBuf,
    pub selector: String,
    pub depth: usize,
    pub siblings: usize,
    pub children: usize,
    pub ref_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GraphShowOutput {
    pub payload: Value,
    pub selector: String,
    pub lineage_names: Vec<String>,
    pub details: GraphNodeDetails,
    pub siblings: Vec<String>,
    pub children: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum GraphNodeDetails {
    Dir {
        parent: Option<String>,
        dirs: usize,
        files: usize,
    },
    File {
        extension: Option<String>,
        parent: Option<String>,
        leaves: usize,
    },
    Leaf {
        kind: String,
        lines: Option<(u32, u32)>,
        parent: Option<String>,
        source: String,
    },
}

#[derive(Debug, Clone)]
pub struct GraphSearchOptions {
    pub data_root: PathBuf,
    pub query: String,
    pub node_types: Vec<String>,
    pub prefix: Option<String>,
    pub limit: usize,
    pub ref_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GraphSearchOutput {
    pub selectors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GraphHistoryOptions {
    pub selector: String,
    pub ref_name: Option<String>,
}

pub fn default_orbitignore_template() -> String {
    orbit_knowledge::default_orbitignore_template()
}

pub fn resolve_graph_build(options: GraphBuildOptions) -> Result<ResolvedGraphBuild, OrbitError> {
    let repo_path = options
        .repo_override
        .unwrap_or_else(|| repo_from_data_root(&options.data_root));
    let output_dir = options.data_root.join("knowledge");
    let mode = if options.incremental {
        "update"
    } else {
        "build"
    };
    Ok(ResolvedGraphBuild {
        mode,
        repo_path,
        output_dir,
        incremental: options.incremental,
        ref_name: parse_ref_name(options.ref_name)?,
    })
}

pub fn run_resolved_graph_build(
    resolved: ResolvedGraphBuild,
) -> Result<GraphBuildOutput, OrbitError> {
    let config = BuildConfig {
        repo_path: resolved.repo_path,
        output_dir: resolved.output_dir.clone(),
        incremental: resolved.incremental,
        ref_name: resolved.ref_name,
    };

    let ctx = orbit_knowledge::pipeline::run_build(config).map_err(|error| {
        OrbitError::Execution(format!("knowledge {} failed: {error}", resolved.mode))
    })?;

    Ok(GraphBuildOutput {
        mode: resolved.mode,
        output_dir: ctx.output_dir,
        dirs: ctx.graph.dirs.len(),
        files: ctx.graph.files.len(),
        leaves: ctx.graph.leaves.len(),
    })
}

pub fn build_graph(options: GraphBuildOptions) -> Result<GraphBuildOutput, OrbitError> {
    run_resolved_graph_build(resolve_graph_build(options)?)
}

pub fn show_graph(options: GraphShowOptions) -> Result<GraphShowOutput, OrbitError> {
    let graph = load_graph(&options.data_root, options.ref_name.as_deref())?;
    let service = GraphContextService::new(&graph);

    let selector: Selector = options
        .selector
        .parse()
        .map_err(|error| OrbitError::InvalidInput(format!("{error}")))?;

    let node = service
        .resolve_selector(&selector)
        .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;

    let context = service
        .bounded_context(node.id(), options.depth, options.siblings, options.children)
        .map_err(|error| OrbitError::Execution(error.to_string()))?;

    Ok(show_output_from_context(&service, &context))
}

pub fn search_graph(options: GraphSearchOptions) -> Result<GraphSearchOutput, OrbitError> {
    let graph = load_graph(&options.data_root, options.ref_name.as_deref())?;
    let service = GraphContextService::new(&graph);

    let type_refs: Vec<&str> = options.node_types.iter().map(String::as_str).collect();
    let node_types = if type_refs.is_empty() {
        None
    } else {
        Some(type_refs.as_slice())
    };

    let selectors = service
        .search(
            &options.query,
            node_types,
            options.prefix.as_deref(),
            None,
            options.limit,
        )
        .into_iter()
        .map(|node| service.selector_for_node(node))
        .collect();

    Ok(GraphSearchOutput { selectors })
}

pub fn history_graph(options: GraphHistoryOptions) -> Result<(), OrbitError> {
    let _selector: Selector = options
        .selector
        .parse()
        .map_err(|error| OrbitError::InvalidInput(format!("{error}")))?;
    parse_ref_name(options.ref_name)?;

    Err(OrbitError::InvalidInput(
        REMOVED_GRAPH_HISTORY_MESSAGE.to_string(),
    ))
}

pub(crate) fn node_context_payload(
    service: &GraphContextService<'_>,
    context: &orbit_knowledge::service::NodeContext<'_>,
) -> Value {
    let node = context.node;

    let lineage: Vec<String> = context
        .lineage
        .iter()
        .map(|node| service.selector_for_node(*node))
        .collect();
    let siblings: Vec<String> = context
        .siblings
        .iter()
        .map(|node| service.selector_for_node(*node))
        .collect();
    let children: Vec<String> = context
        .children
        .iter()
        .map(|node| service.selector_for_node(*node))
        .collect();

    let mut value = json!({
        "selector": service.selector_for_node(node),
        "lineage": lineage,
        "siblings": siblings,
        "children": children,
    });

    match node {
        GraphNodeRef::Leaf(leaf) => {
            let obj = value.as_object_mut().expect("node context payload object");
            obj.insert("source".to_string(), json!(leaf.source));
            obj.insert("lines".to_string(), json!([leaf.start_line, leaf.end_line]));
        }
        GraphNodeRef::File(file) => {
            let obj = value.as_object_mut().expect("node context payload object");
            if !file.imports.is_empty() {
                obj.insert("imports".to_string(), json!(file.imports));
            }
            if !file.exports.is_empty() {
                obj.insert("exports".to_string(), json!(file.exports));
            }
            if !file.re_exports.is_empty() {
                obj.insert("re_exports".to_string(), json!(file.re_exports));
            }
        }
        GraphNodeRef::Dir(_) => {}
    }

    value
}

fn load_graph(data_root: &Path, explicit_ref: Option<&str>) -> Result<CodebaseGraphV1, OrbitError> {
    let knowledge_dir = data_root.join("knowledge");
    let repo_path = repo_from_data_root(data_root);
    let service = TaskGraphService::new(knowledge_dir, TaskGraphScope::default());
    service.read_graph(Some(&repo_path), false, explicit_ref)
}

fn repo_from_data_root(data_root: &Path) -> PathBuf {
    data_root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn show_output_from_context(
    service: &GraphContextService<'_>,
    context: &orbit_knowledge::service::NodeContext<'_>,
) -> GraphShowOutput {
    let node = context.node;
    let selector = service.selector_for_node(node);
    let lineage_names = context
        .lineage
        .iter()
        .map(|node| node.base().name.clone())
        .chain(std::iter::once(node.base().name.clone()))
        .collect();
    let siblings = context
        .siblings
        .iter()
        .map(|node| service.selector_for_node(*node))
        .collect();
    let children = context
        .children
        .iter()
        .map(|node| service.selector_for_node(*node))
        .collect();
    let details = node_details(service, node);
    let payload = node_context_payload(service, context);

    GraphShowOutput {
        payload,
        selector,
        lineage_names,
        details,
        siblings,
        children,
    }
}

fn node_details(service: &GraphContextService<'_>, node: GraphNodeRef<'_>) -> GraphNodeDetails {
    let parent = node
        .parent_id()
        .and_then(|parent_id| service.navigator().get_node(parent_id).ok())
        .map(|parent| service.selector_for_node(parent));

    match node {
        GraphNodeRef::Dir(dir) => GraphNodeDetails::Dir {
            parent,
            dirs: dir.dir_children.len(),
            files: dir.file_children.len(),
        },
        GraphNodeRef::File(file) => GraphNodeDetails::File {
            extension: file.extension.clone(),
            parent,
            leaves: file.leaf_children.len(),
        },
        GraphNodeRef::Leaf(leaf) => GraphNodeDetails::Leaf {
            kind: leaf.kind.to_string(),
            lines: leaf.start_line.zip(leaf.end_line),
            parent,
            source: leaf.source.clone(),
        },
    }
}

fn parse_ref_name(ref_name: Option<String>) -> Result<Option<RefName>, OrbitError> {
    ref_name
        .filter(|value| !value.trim().is_empty())
        .map(RefName::new)
        .transpose()
        .map_err(|error| OrbitError::InvalidInput(error.to_string()))
}
