use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use comfy_table::Cell;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_knowledge::graph::navigator::GraphNodeRef;
use orbit_knowledge::graph::nodes::CodebaseGraphV1;
use orbit_knowledge::graph::object_store::RefName;
use orbit_knowledge::pipeline::context::BuildConfig;
use orbit_knowledge::service::GraphContextService;
use orbit_knowledge::{Selector, TaskGraphScope, TaskGraphService};
use serde_json::json;

use crate::command::Execute;
use crate::output::table::{add_single_line_row, build_table};

#[derive(Args)]
#[command(about = "Build and query the knowledge graph")]
pub struct GraphCommand {
    #[command(subcommand)]
    pub subcommand: GraphSubcommand,
}

#[derive(Subcommand)]
pub enum GraphSubcommand {
    /// Build the knowledge graph from scratch
    Build(GraphBuildArgs),
    /// Incrementally update the knowledge graph
    Update(GraphUpdateArgs),
    /// Show a node and its context
    Show(GraphShowArgs),
    /// Search nodes by name or location
    Search(GraphSearchArgs),
}

#[derive(Args)]
pub struct GraphBuildArgs {
    /// Repository root (defaults to current working directory)
    #[arg(long)]
    pub repo: Option<PathBuf>,

    /// Knowledge-graph ref name (defaults to the current git branch)
    #[arg(long = "ref")]
    pub ref_name: Option<String>,
}

#[derive(Args)]
pub struct GraphUpdateArgs {
    /// Repository root (defaults to current working directory)
    #[arg(long)]
    pub repo: Option<PathBuf>,

    /// Knowledge-graph ref name (defaults to the current git branch)
    #[arg(long = "ref")]
    pub ref_name: Option<String>,
}

#[derive(Args)]
pub struct GraphShowArgs {
    /// Selector (e.g. file:src/lib.rs, symbol:src/lib.rs#hello:function, dir:src)
    pub selector: String,

    /// Ancestor depth
    #[arg(long, default_value = "2")]
    pub depth: usize,

    /// Max siblings to display
    #[arg(long, default_value = "3")]
    pub siblings: usize,

    /// Max children to display
    #[arg(long, default_value = "5")]
    pub children: usize,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Knowledge-graph ref name (defaults to the current git branch)
    #[arg(long = "ref")]
    pub ref_name: Option<String>,
}

#[derive(Args)]
pub struct GraphSearchArgs {
    /// Search query (matches name or location)
    pub query: String,

    /// Filter by node type (dir, file, symbol); can be repeated
    #[arg(long = "type", value_name = "TYPE")]
    pub node_types: Vec<String>,

    /// Filter by location prefix
    #[arg(long)]
    pub prefix: Option<String>,

    /// Max results
    #[arg(long, default_value = "20")]
    pub limit: usize,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Knowledge-graph ref name (defaults to the current git branch)
    #[arg(long = "ref")]
    pub ref_name: Option<String>,
}

impl Execute for GraphCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self.subcommand {
            GraphSubcommand::Build(args) => args.execute(runtime),
            GraphSubcommand::Update(args) => args.execute(runtime),
            GraphSubcommand::Show(args) => args.execute(runtime),
            GraphSubcommand::Search(args) => args.execute(runtime),
        }
    }
}

impl Execute for GraphBuildArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        run_pipeline(runtime, self.repo, self.ref_name, false)
    }
}

impl Execute for GraphUpdateArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        run_pipeline(runtime, self.repo, self.ref_name, true)
    }
}

impl Execute for GraphShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let graph = load_graph(runtime, self.ref_name.as_deref())?;
        let svc = GraphContextService::new(&graph);

        let selector: Selector = self
            .selector
            .parse()
            .map_err(|e| OrbitError::InvalidInput(format!("{e}")))?;

        let node = svc
            .resolve_selector(&selector)
            .map_err(|e| OrbitError::InvalidInput(e.to_string()))?;

        let ctx = svc
            .bounded_context(node.id(), self.depth, self.siblings, self.children)
            .map_err(|e| OrbitError::Execution(e.to_string()))?;

        if self.json {
            let value = node_context_to_json(&svc, &ctx);
            crate::output::json::print_pretty(&value)?;
        } else {
            print_node_context(&svc, &ctx);
        }

        Ok(())
    }
}

impl Execute for GraphSearchArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let graph = load_graph(runtime, self.ref_name.as_deref())?;
        let svc = GraphContextService::new(&graph);

        let type_refs: Vec<&str> = self.node_types.iter().map(String::as_str).collect();
        let node_types = if type_refs.is_empty() {
            None
        } else {
            Some(type_refs.as_slice())
        };

        let results = svc.search(
            &self.query,
            node_types,
            self.prefix.as_deref(),
            None,
            self.limit,
        );

        if self.json {
            let items: Vec<String> = results.iter().map(|n| svc.selector_for_node(*n)).collect();
            crate::output::json::print_pretty(&json!(items))?;
        } else if results.is_empty() {
            println!("No results found.");
        } else {
            let mut table = build_table(&["SELECTOR"]);
            for node in &results {
                add_single_line_row(&mut table, vec![Cell::new(svc.selector_for_node(*node))]);
            }
            println!("{table}");
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_graph(
    runtime: &OrbitRuntime,
    explicit_ref: Option<&str>,
) -> Result<CodebaseGraphV1, OrbitError> {
    let data_root = runtime.data_root();
    let knowledge_dir = data_root.join("knowledge");
    let repo_path = data_root.parent().unwrap_or_else(|| Path::new("."));
    let service = TaskGraphService::new(knowledge_dir, TaskGraphScope::default());
    service.read_graph(Some(repo_path), false, explicit_ref)
}

fn run_pipeline(
    runtime: &OrbitRuntime,
    repo_override: Option<PathBuf>,
    ref_name: Option<String>,
    incremental: bool,
) -> Result<(), OrbitError> {
    let data_root = runtime.data_root();
    let repo_path = repo_override.unwrap_or_else(|| {
        data_root
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    });
    let output_dir = data_root.join("knowledge");

    let mode = if incremental { "update" } else { "build" };
    eprintln!("knowledge {mode}: scanning {}", repo_path.display());

    let config = BuildConfig {
        repo_path,
        output_dir,
        incremental,
        ref_name: parse_ref_name(ref_name)?,
    };

    let ctx = orbit_knowledge::pipeline::run_build(config)
        .map_err(|e| OrbitError::Execution(format!("knowledge {mode} failed: {e}")))?;

    eprintln!(
        "knowledge {mode}: {} dirs, {} files, {} leaves",
        ctx.graph.dirs.len(),
        ctx.graph.files.len(),
        ctx.graph.leaves.len(),
    );
    eprintln!("knowledge {mode}: written to {}", ctx.output_dir.display());

    Ok(())
}

fn parse_ref_name(ref_name: Option<String>) -> Result<Option<RefName>, OrbitError> {
    ref_name
        .filter(|value| !value.trim().is_empty())
        .map(RefName::new)
        .transpose()
        .map_err(|error| OrbitError::InvalidInput(error.to_string()))
}

fn node_context_to_json(
    svc: &GraphContextService<'_>,
    ctx: &orbit_knowledge::service::NodeContext<'_>,
) -> serde_json::Value {
    let node = ctx.node;

    let lineage: Vec<String> = ctx
        .lineage
        .iter()
        .map(|n| svc.selector_for_node(*n))
        .collect();
    let siblings: Vec<String> = ctx
        .siblings
        .iter()
        .map(|n| svc.selector_for_node(*n))
        .collect();
    let children: Vec<String> = ctx
        .children
        .iter()
        .map(|n| svc.selector_for_node(*n))
        .collect();

    let mut value = json!({
        "selector": svc.selector_for_node(node),
        "lineage": lineage,
        "siblings": siblings,
        "children": children,
    });

    match node {
        GraphNodeRef::Leaf(l) => {
            let obj = value.as_object_mut().unwrap();
            obj.insert("source".to_string(), json!(l.source));
            obj.insert("lines".to_string(), json!([l.start_line, l.end_line]));
        }
        GraphNodeRef::File(f) => {
            let obj = value.as_object_mut().unwrap();
            if !f.imports.is_empty() {
                obj.insert("imports".to_string(), json!(f.imports));
            }
            if !f.exports.is_empty() {
                obj.insert("exports".to_string(), json!(f.exports));
            }
        }
        GraphNodeRef::Dir(_) => {}
    }

    value
}

fn print_node_context(
    svc: &GraphContextService<'_>,
    ctx: &orbit_knowledge::service::NodeContext<'_>,
) {
    let node = ctx.node;
    let sel = svc.selector_for_node(node);

    println!("{sel}");
    println!();

    // Lineage breadcrumb
    if !ctx.lineage.is_empty() {
        let breadcrumb: Vec<&str> = ctx
            .lineage
            .iter()
            .map(|n| n.base().name.as_str())
            .chain(std::iter::once(node.base().name.as_str()))
            .collect();
        println!("  Lineage: {}", breadcrumb.join(" > "));
    }

    // Type-specific details
    match node {
        GraphNodeRef::Dir(d) => {
            println!("  Type:    dir");
            if let Some(pid) = node.parent_id()
                && let Ok(parent) = svc.navigator().get_node(pid)
            {
                println!("  Parent:  {}", svc.selector_for_node(parent));
            }
            println!(
                "  Dirs:    {}  Files: {}",
                d.dir_children.len(),
                d.file_children.len()
            );
        }
        GraphNodeRef::File(f) => {
            println!("  Type:    file");
            if let Some(ext) = &f.extension {
                println!("  Ext:     {ext}");
            }
            if let Some(pid) = node.parent_id()
                && let Ok(parent) = svc.navigator().get_node(pid)
            {
                println!("  Parent:  {}", svc.selector_for_node(parent));
            }
            println!("  Leaves:  {}", f.leaf_children.len());
        }
        GraphNodeRef::Leaf(l) => {
            println!("  Kind:    {}", l.kind);
            if let (Some(s), Some(e)) = (l.start_line, l.end_line) {
                println!("  Lines:   {s}..{e}");
            }
            if let Some(pid) = node.parent_id()
                && let Ok(parent) = svc.navigator().get_node(pid)
            {
                println!("  Parent:  {}", svc.selector_for_node(parent));
            }
            if !l.source.is_empty() {
                println!();
                println!("  Source:");
                for line in l.source.lines() {
                    println!("    {line}");
                }
            }
        }
    }

    // Siblings
    println!();
    if ctx.siblings.is_empty() {
        println!("  Siblings: (none)");
    } else {
        println!("  Siblings ({}):", ctx.siblings.len());
        for sib in &ctx.siblings {
            println!("    {}", svc.selector_for_node(*sib));
        }
    }

    // Children (skip for leaf nodes — methods are accessible via search/selectors)
    if !matches!(node, GraphNodeRef::Leaf(_)) {
        println!();
        if ctx.children.is_empty() {
            println!("  Children: (none)");
        } else {
            println!("  Children ({}):", ctx.children.len());
            for child in &ctx.children {
                println!("    {}", svc.selector_for_node(*child));
            }
        }
    }
}
