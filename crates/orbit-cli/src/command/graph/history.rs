use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::support::run_history_query;

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
}

impl Execute for GraphHistoryArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        run_history_query(runtime, &self.selector, self.ref_name.as_deref())
    }
}
