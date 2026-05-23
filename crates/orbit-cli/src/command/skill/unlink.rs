use clap::Args;
use orbit_core::command::init::UnlinkResult;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
pub struct SkillUnlinkArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for SkillUnlinkArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let result = orbit_core::command::init::unlink_skills(&runtime.global_root())?;
        if self.json {
            crate::output::json::print_pretty(&unlink_result_json(&result))
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
            Ok(())
        }
    }
}

fn unlink_result_json(result: &UnlinkResult) -> Value {
    json!({
        "removed_count": result.removed_count,
        "cleaned_dirs": result.cleaned_dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    })
}
