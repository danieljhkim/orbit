use clap::Args;
use orbit_core::OrbitRuntime;
use serde_json::{Map, Value};

use crate::command::{CommandOut, Execute, Payload};
use crate::output::color::Domain;

use super::support::{field_list, field_str};

#[derive(Args)]
pub struct AdrListArgs {
    /// Filter by status: proposed | accepted | superseded | deleted
    #[arg(long)]
    pub status: Option<String>,
    /// Filter by owner (e.g. `claude`)
    #[arg(long)]
    pub owner: Option<String>,
    /// Filter by feature folder name in `related_features`
    #[arg(long)]
    pub feature: Option<String>,
    /// Filter by Orbit task ID in `related_tasks`
    #[arg(long = "task-id")]
    pub task_id: Option<String>,
    /// Filter by legacy ID alias in `legacy_ids`
    #[arg(long = "legacy-id")]
    pub legacy_id: Option<String>,
    /// Filter by free-form ADR tag (case-insensitive)
    #[arg(long)]
    pub tag: Option<String>,
    /// Filter by repo-relative path contained by any ADR `paths` glob
    #[arg(long)]
    pub path: Option<String>,
    /// When set, return only ADRs with `legacy_validation = warned`
    #[arg(long = "validation-warned")]
    pub validation_warned: bool,
    /// Include allocated federated ADRs whose body files are not locally readable as remote stubs
    #[arg(long = "include-remote")]
    pub include_remote: bool,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AdrListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let mut input = Map::new();
        if let Some(value) = self.status {
            input.insert("status".to_string(), Value::String(value));
        }
        if let Some(value) = self.owner {
            input.insert("owner".to_string(), Value::String(value));
        }
        if let Some(value) = self.feature {
            input.insert("feature".to_string(), Value::String(value));
        }
        if let Some(value) = self.task_id {
            input.insert("task_id".to_string(), Value::String(value));
        }
        if let Some(value) = self.legacy_id {
            input.insert("legacy_id".to_string(), Value::String(value));
        }
        if let Some(value) = self.tag {
            input.insert("tag".to_string(), Value::String(value));
        }
        if let Some(value) = self.path {
            input.insert("path".to_string(), Value::String(value));
        }
        if self.validation_warned {
            input.insert("validation_warned".to_string(), Value::Bool(true));
        }
        if self.include_remote {
            input.insert("include_remote".to_string(), Value::Bool(true));
        }

        // ORB-00289: `runtime.run_tool` bypasses `ensure_tool_agent_facing`
        // (which now rejects `orbit.adr.list` on the agent surface) while
        // still going through the tool's input parsing and filter
        // semantics. This is the same path used by `orbit-dashboard`'s
        // `run_adr_tool` helper.
        let value = runtime.run_tool("orbit.adr.list", Value::Object(input))?;

        // The tool already returns the canonical ADR envelope shape, so it is
        // the payload unchanged. Until ORB-10586 this command printed that
        // shape unconditionally and ignored its own `--json`; the mode now
        // decides, and the human form is the list view below.
        let _ = self.json;
        let Some(records) = value.as_array() else {
            return Ok(Payload::document(value).into());
        };

        use crate::output::table::{Column, Table};
        // `orbit adr show <id>` prints the untruncated body of any row.
        let mut table = Table::new(vec![
            Column::new("ID").fixed(),
            Column::new("STATUS").fixed(),
            Column::new("OWNER").fixed(),
            Column::new("TITLE"),
            Column::new("FEATURES"),
            Column::new("UPDATED").fixed(),
        ])
        .empty_message("no ADRs matching the given filters");
        for record in records {
            table.add_row(vec![
                comfy_table::Cell::new(field_str(record, "id")),
                crate::output::color::cell(&field_str(record, "status"), Domain::TaskStatus),
                comfy_table::Cell::new(field_str(record, "owner")),
                comfy_table::Cell::new(field_str(record, "title")),
                comfy_table::Cell::new(field_list(record, "related_features")),
                comfy_table::Cell::new(field_str(record, "last_updated")),
            ]);
        }
        Ok(Payload::list(records.clone(), table).into())
    }
}
