use clap::Args;
use orbit_core::OrbitRuntime;
use orbit_core::bootstrap::init::UnlinkResult;
use serde_json::{Value, json};

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

#[derive(Args)]
pub struct SkillUnlinkArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for SkillUnlinkArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let result = orbit_core::bootstrap::init::unlink_skills(&runtime.global_root())?;
        if self.json {
            Ok(Payload::document(unlink_result_json(&result)).into())
        } else {
            if result.removed_count == 0 {
                println!("No skill symlinks found to remove.");
            } else {
                println!("Removed {} skill symlink(s).", result.removed_count);
            }
            if !result.cleaned_dirs.is_empty() {
                println!("Cleaned up empty directories:");
                for dir in &result.cleaned_dirs {
                    println!("  {}", dir.display());
                }
            }
            Ok(CommandOutput::Silent)
        }
    }
}

fn unlink_result_json(result: &UnlinkResult) -> Value {
    json!({
        "removed_count": result.removed_count,
        "cleaned_dirs": result.cleaned_dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    })
}
