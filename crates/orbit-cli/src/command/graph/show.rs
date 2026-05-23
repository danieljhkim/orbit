use clap::Args;
use orbit_core::command::graph as graph_service;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::support::print_node_context;

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
