use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::support::tool_status;

#[derive(Args)]
pub struct ToolShowArgs {
    /// Tool name
    pub name: String,
}

impl Execute for ToolShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let tool = runtime.show_tool(&self.name)?;

        use crate::output::color::{bold, job_state_color};
        println!("{} {}", bold("Name:"), tool.name);
        println!("{} {}", bold("Description:"), tool.description);
        println!(
            "{} {}",
            bold("Builtin:"),
            if tool.builtin { "yes" } else { "no" }
        );
        println!(
            "{} {}",
            bold("Status:"),
            job_state_color(tool_status(&tool))
        );

        if tool.parameters.is_empty() {
            println!("{} (none)", bold("Parameters:"));
        } else {
            println!("{}", bold("Parameters:"));
            let mut table =
                crate::output::table::build_table(&["NAME", "TYPE", "REQUIRED", "DESCRIPTION"]);
            for p in &tool.parameters {
                let req = if p.required { "required" } else { "optional" };
                table.add_row(vec![
                    p.name.clone(),
                    p.param_type.clone(),
                    req.to_string(),
                    p.description.clone(),
                ]);
            }
            println!("{table}");
        }

        Ok(())
    }
}
