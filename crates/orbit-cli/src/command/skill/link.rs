use clap::Args;
use orbit_core::OrbitRuntime;
use orbit_core::command::init::LinkResult;
use serde_json::{Value, json};

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

#[derive(Args)]
pub struct SkillLinkArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for SkillLinkArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let result = orbit_core::command::init::link_skills(&runtime.global_root())?;
        if self.json {
            Ok(Payload::document(link_result_json(&result)).into())
        } else {
            if result.linked_count == 0 {
                println!("Skill symlinks are already up to date.");
            } else {
                println!("Linked {} skill(s) in:", result.linked_count);
                for root in &result.roots {
                    println!("  {}", root.display());
                }
            }
            Ok(CommandOutput::Silent)
        }
    }
}

fn link_result_json(result: &LinkResult) -> Value {
    json!({
        "linked_count": result.linked_count,
        "roots": result.roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    })
}
