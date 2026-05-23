use clap::Args;
use comfy_table::Cell;
use orbit_core::command::graph as graph_service;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::Execute;
use crate::output::table::{add_single_line_row, build_table};

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
