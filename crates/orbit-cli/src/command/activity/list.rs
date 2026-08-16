use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::{CommandOut, Execute, Payload};

#[derive(Args)]
pub struct ActivityListArgs {
    #[arg(long)]
    pub json: bool,
    /// Output signal-tier JSON (id, type, description only)
    #[arg(long)]
    pub ops: bool,
}

impl Execute for ActivityListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let catalog = runtime
            .v2_activity_catalog()
            .map_err(|err| OrbitError::Store(format!("v2 activity catalog: {err}")))?;

        // `--ops` narrows the record shape only; every mode lists the same
        // activities.
        let values: Vec<Value> = if self.ops {
            catalog
                .names()
                .filter_map(|name| catalog.get(name).map(|spec| v2_signal_json(name, spec)))
                .collect()
        } else {
            catalog
                .names()
                .filter_map(|name| catalog.get(name).map(|spec| v2_full_json(name, spec)))
                .collect()
        };

        {
            use crate::output::table::{Column, Table};
            // No `orbit activity show` exists yet, so a machine-readable mode
            // is the only untruncated view of a description. Tracked in
            // docs/design/terminal-interface/references/detail-commands.md.
            let mut table = Table::new(vec![
                Column::new("ID").fixed(),
                Column::new("TYPE").fixed(),
                Column::new("DESCRIPTION"),
            ])
            .empty_message("no activities registered");
            for name in catalog.names() {
                use comfy_table::Cell;
                let Some(spec) = catalog.get(name) else {
                    continue;
                };
                table.add_row(vec![
                    Cell::new(name),
                    Cell::new(v2_type_label(spec)),
                    Cell::new(&spec.description),
                ]);
            }
            Ok(Payload::list(values, table).into())
        }
    }
}

fn v2_type_label(spec: &orbit_types::workflow::activity_job::ActivityV2) -> &'static str {
    use orbit_types::workflow::activity_job::ActivityV2Spec;
    match &spec.spec {
        ActivityV2Spec::AgentLoop(_) => "agent_loop",
        ActivityV2Spec::Deterministic(_) => "deterministic",
    }
}

fn v2_full_json(name: &str, spec: &orbit_types::workflow::activity_job::ActivityV2) -> Value {
    json!({
        "id": name,
        "type": v2_type_label(spec),
        "description": spec.description,
        "input_schema_json": spec.input_schema_json,
        "output_schema_json": spec.output_schema_json,
        "fsProfile": spec.fs_profile,
        "schemaVersion": 2,
    })
}

fn v2_signal_json(name: &str, spec: &orbit_types::workflow::activity_job::ActivityV2) -> Value {
    json!({
        "id": name,
        "type": v2_type_label(spec),
        "description": spec.description,
    })
}
