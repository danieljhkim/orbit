use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
pub struct ActivityListArgs {
    #[arg(long)]
    pub json: bool,
    /// Output signal-tier JSON (id, type, description only)
    #[arg(long)]
    pub ops: bool,
}

impl Execute for ActivityListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let catalog = runtime
            .v2_activity_catalog()
            .map_err(|err| OrbitError::Store(format!("v2 activity catalog: {err}")))?;

        if self.ops {
            let values: Vec<Value> = catalog
                .names()
                .filter_map(|name| catalog.get(name).map(|spec| v2_signal_json(name, spec)))
                .collect();
            crate::output::json::print_pretty(&Value::Array(values))
        } else if self.json {
            let values: Vec<Value> = catalog
                .names()
                .filter_map(|name| catalog.get(name).map(|spec| v2_full_json(name, spec)))
                .collect();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            let mut table = crate::output::table::build_table(&["ID", "TYPE", "DESCRIPTION"]);
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
            println!("{table}");
            Ok(())
        }
    }
}

fn v2_type_label(spec: &orbit_common::types::activity_job::ActivityV2) -> &'static str {
    use orbit_common::types::activity_job::ActivityV2Spec;
    match &spec.spec {
        ActivityV2Spec::AgentLoop(_) => "agent_loop",
        ActivityV2Spec::Groundhog(_) => "groundhog",
        ActivityV2Spec::Deterministic(_) => "deterministic",
    }
}

fn v2_full_json(name: &str, spec: &orbit_common::types::activity_job::ActivityV2) -> Value {
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

fn v2_signal_json(name: &str, spec: &orbit_common::types::activity_job::ActivityV2) -> Value {
    json!({
        "id": name,
        "type": v2_type_label(spec),
        "description": spec.description,
    })
}
