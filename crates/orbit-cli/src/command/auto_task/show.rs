use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::{CommandOut, Execute, Payload};

use super::output::definition_to_json;

#[derive(Args)]
pub struct AutoTaskShowArgs {
    /// Definition name
    pub name: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AutoTaskShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let definition = runtime.auto_task_show(&self.name)?.ok_or_else(|| {
            OrbitError::InvalidInput(format!("no such auto-task '{}'", self.name))
        })?;

        let doc = definition_to_json(&definition);

        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{} ({})",
            definition.name,
            if definition.enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
        if !definition.description.is_empty() {
            let _ = writeln!(out, "  {}", definition.description);
        }
        let _ = writeln!(
            out,
            "  schedule: {}",
            super::output::schedule_summary(&definition)
        );
        let _ = writeln!(out, "  dedupe: {:?}", definition.dedupe);
        let _ = writeln!(out, "  template: {}", definition.template.title);
        Ok(Payload::detail(doc, out).into())
    }
}
