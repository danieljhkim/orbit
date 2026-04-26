use std::path::PathBuf;

use clap::{Args, Subcommand};
use comfy_table::Cell;
use orbit_core::command::graph as graph_service;
use orbit_core::{OrbitError, OrbitRuntime};
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
    /// Query task-ID history for a knowledge-graph selector
    #[command(
        long_about = "Query task-ID history for a knowledge-graph selector.\n\n\
        Prefers the graph-backed path (`task_ids` on the node plus the \
        task-commits sidecar); falls back to a `git log` scan when either is \
        missing. The fallback emits a stderr warning and does not follow \
        renames or moves.\n\n\
        Capture-group convention (T20260426-0507): if the configured \
        `--task-id-pattern` regex contains at least one capture group, group 1 \
        is the task ID. Otherwise the whole match is the task ID. The Orbit \
        default `\\[(T\\d{8}-\\d{4}(?:-\\d+)?)\\]` strips the surrounding \
        brackets via group 1.\n\n\
        Pattern resolution order: `--task-id-pattern` flag > workspace config \
        `knowledge.task_id_pattern` > Orbit default."
    )]
    History(GraphHistoryArgs),
}

#[derive(Args)]
pub struct GraphBuildArgs {
    /// Repository root (defaults to current working directory)
    #[arg(long)]
    pub repo: Option<PathBuf>,

    /// Knowledge-graph ref name (defaults to the current git branch)
    #[arg(long = "ref")]
    pub ref_name: Option<String>,

    /// Task-ID extraction regex override. Falls back to the workspace
    /// `knowledge.task_id_pattern` config and then the Orbit default.
    #[arg(long = "task-id-pattern")]
    pub task_id_pattern: Option<String>,
}

#[derive(Args)]
pub struct GraphUpdateArgs {
    /// Repository root (defaults to current working directory)
    #[arg(long)]
    pub repo: Option<PathBuf>,

    /// Knowledge-graph ref name (defaults to the current git branch)
    #[arg(long = "ref")]
    pub ref_name: Option<String>,

    /// Task-ID extraction regex override. Falls back to the workspace
    /// `knowledge.task_id_pattern` config and then the Orbit default.
    #[arg(long = "task-id-pattern")]
    pub task_id_pattern: Option<String>,
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
pub struct GraphHistoryArgs {
    /// Selector to query (e.g. `file:src/lib.rs`,
    /// `symbol:src/lib.rs#hello:function`, `dir:src`).
    pub selector: String,

    /// Output as JSON.
    #[arg(long)]
    pub json: bool,

    /// Knowledge-graph ref name (defaults to the current git branch).
    #[arg(long = "ref")]
    pub ref_name: Option<String>,

    /// Staleness threshold (commits). A warning is emitted to stderr when
    /// HEAD is more than this many commits ahead of the attribution cursor.
    #[arg(long, default_value_t = graph_service::DEFAULT_STALENESS_THRESHOLD)]
    pub staleness_threshold: u64,

    /// Task-ID extraction regex override. Falls back to the workspace
    /// `knowledge.task_id_pattern` config and then the Orbit default.
    #[arg(long = "task-id-pattern")]
    pub task_id_pattern: Option<String>,
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
            GraphSubcommand::History(args) => args.execute(runtime),
        }
    }
}

impl Execute for GraphBuildArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        run_pipeline(
            runtime,
            self.repo,
            self.ref_name,
            self.task_id_pattern,
            false,
        )
    }
}

impl Execute for GraphUpdateArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        run_pipeline(
            runtime,
            self.repo,
            self.ref_name,
            self.task_id_pattern,
            true,
        )
    }
}

impl Execute for GraphHistoryArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        run_history_query(
            runtime,
            &self.selector,
            self.json,
            self.ref_name.as_deref(),
            self.staleness_threshold,
            self.task_id_pattern.as_deref(),
        )
    }
}

impl Execute for GraphShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let output = graph_service::show_graph(graph_service::GraphShowOptions {
            data_root: runtime.data_root(),
            selector: self.selector,
            depth: self.depth,
            siblings: self.siblings,
            children: self.children,
            ref_name: self.ref_name,
        })?;

        if self.json {
            crate::output::json::print_pretty(&output.payload)?;
        } else {
            print_node_context(&output);
        }

        Ok(())
    }
}

impl Execute for GraphSearchArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let output = graph_service::search_graph(graph_service::GraphSearchOptions {
            data_root: runtime.data_root(),
            query: self.query,
            node_types: self.node_types,
            prefix: self.prefix,
            limit: self.limit,
            ref_name: self.ref_name,
        })?;

        if self.json {
            crate::output::json::print_pretty(&json!(output.selectors))?;
        } else if output.selectors.is_empty() {
            println!("No results found.");
        } else {
            let mut table = build_table(&["SELECTOR"]);
            for selector in &output.selectors {
                add_single_line_row(&mut table, vec![Cell::new(selector)]);
            }
            println!("{table}");
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn run_pipeline(
    runtime: &OrbitRuntime,
    repo_override: Option<PathBuf>,
    ref_name: Option<String>,
    task_id_pattern_flag: Option<String>,
    incremental: bool,
) -> Result<(), OrbitError> {
    let resolved = graph_service::resolve_graph_build(graph_service::GraphBuildOptions {
        data_root: runtime.data_root(),
        repo_override,
        ref_name,
        task_id_pattern_flag,
        task_id_pattern_config: runtime.task_id_pattern().map(ToOwned::to_owned),
        incremental,
    })?;
    eprintln!(
        "knowledge {}: scanning {}",
        resolved.mode,
        resolved.repo_path.display()
    );
    eprintln!(
        "knowledge {}: task-ID pattern {}",
        resolved.mode, resolved.task_id_pattern
    );

    let output = graph_service::run_resolved_graph_build(resolved)?;

    eprintln!(
        "knowledge {}: {} dirs, {} files, {} leaves",
        output.mode, output.dirs, output.files, output.leaves,
    );
    eprintln!(
        "knowledge {}: written to {}",
        output.mode,
        output.output_dir.display()
    );
    Ok(())
}

fn run_history_query(
    runtime: &OrbitRuntime,
    raw_selector: &str,
    as_json: bool,
    explicit_ref: Option<&str>,
    staleness_threshold: u64,
    task_id_pattern_flag: Option<&str>,
) -> Result<(), OrbitError> {
    let output = graph_service::history_graph(graph_service::GraphHistoryOptions {
        data_root: runtime.data_root(),
        selector: raw_selector.to_string(),
        ref_name: explicit_ref.map(ToOwned::to_owned),
        staleness_threshold,
        task_id_pattern_flag: task_id_pattern_flag.map(ToOwned::to_owned),
        task_id_pattern_config: runtime.task_id_pattern().map(ToOwned::to_owned),
    })?;

    for warning in &output.warnings {
        eprintln!("warning: {warning}");
    }

    if as_json {
        crate::output::json::print_pretty(&output.payload)?;
    } else {
        print_history_human(&output);
    }

    Ok(())
}

fn print_history_human(output: &graph_service::GraphHistoryOutput) {
    use crate::output::color::{bold, dimmed};

    println!("{} {}", bold("Selector:"), output.selector);
    println!("{} {}", bold("Source:"), output.source_label);
    if output.structural_conflict {
        println!("{}", bold("Structural conflict: true"));
    }
    if output.task_history.is_empty() {
        println!("{}", dimmed("No task IDs attributed to this node."));
        return;
    }
    for entry in &output.task_history {
        println!("{} {}", bold("Task:"), entry.task_id);
        if entry.commits.is_empty() {
            println!("  {}", dimmed("(no commits found in sidecar)"));
            continue;
        }
        for commit in &entry.commits {
            let short_sha: String = commit.sha.chars().take(12).collect();
            println!("  {} {} {}", short_sha, commit.date, commit.summary);
        }
    }
}

fn print_node_context(output: &graph_service::GraphShowOutput) {
    println!("{}", output.selector);
    println!();

    if !output.lineage_names.is_empty() {
        println!("  Lineage: {}", output.lineage_names.join(" > "));
    }

    match &output.details {
        graph_service::GraphNodeDetails::Dir {
            parent,
            dirs,
            files,
        } => {
            println!("  Type:    dir");
            if let Some(parent) = parent {
                println!("  Parent:  {parent}");
            }
            println!("  Dirs:    {dirs}  Files: {files}");
        }
        graph_service::GraphNodeDetails::File {
            extension,
            parent,
            leaves,
        } => {
            println!("  Type:    file");
            if let Some(extension) = extension {
                println!("  Ext:     {extension}");
            }
            if let Some(parent) = parent {
                println!("  Parent:  {parent}");
            }
            println!("  Leaves:  {leaves}");
        }
        graph_service::GraphNodeDetails::Leaf {
            kind,
            lines,
            parent,
            source,
        } => {
            println!("  Kind:    {kind}");
            if let Some((start, end)) = lines {
                println!("  Lines:   {start}..{end}");
            }
            if let Some(parent) = parent {
                println!("  Parent:  {parent}");
            }
            if !source.is_empty() {
                println!();
                println!("  Source:");
                for line in source.lines() {
                    println!("    {line}");
                }
            }
        }
    }

    println!();
    if output.siblings.is_empty() {
        println!("  Siblings: (none)");
    } else {
        println!("  Siblings ({}):", output.siblings.len());
        for sibling in &output.siblings {
            println!("    {sibling}");
        }
    }

    if !matches!(output.details, graph_service::GraphNodeDetails::Leaf { .. }) {
        println!();
        if output.children.is_empty() {
            println!("  Children: (none)");
        } else {
            println!("  Children ({}):", output.children.len());
            for child in &output.children {
                println!("    {child}");
            }
        }
    }
}
