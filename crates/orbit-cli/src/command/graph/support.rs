use std::path::PathBuf;

use orbit_core::command::graph as graph_service;
use orbit_core::{OrbitError, OrbitRuntime};

pub(super) fn run_pipeline(
    runtime: &OrbitRuntime,
    repo_override: Option<PathBuf>,
    ref_name: Option<String>,
    incremental: bool,
) -> Result<(), OrbitError> {
    let resolved = graph_service::resolve_graph_build(graph_service::GraphBuildOptions {
        data_root: runtime.data_root(),
        repo_override,
        ref_name,
        incremental,
    })?;
    eprintln!(
        "knowledge {}: scanning {}",
        resolved.mode,
        resolved.repo_path.display()
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

pub(super) fn run_history_query(
    runtime: &OrbitRuntime,
    raw_selector: &str,
    explicit_ref: Option<&str>,
) -> Result<(), OrbitError> {
    let _ = runtime;
    graph_service::history_graph(graph_service::GraphHistoryOptions {
        selector: raw_selector.to_string(),
        ref_name: explicit_ref.map(ToOwned::to_owned),
    })?;
    Ok(())
}

pub(super) fn print_node_context(output: &graph_service::GraphShowOutput) {
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
