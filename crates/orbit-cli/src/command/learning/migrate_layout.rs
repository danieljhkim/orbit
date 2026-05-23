use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime, migrate_learning_layout_at};
use serde_json::json;

use crate::command::Execute;

#[derive(Args)]
pub struct LearningMigrateLayoutArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl LearningMigrateLayoutArgs {
    pub fn execute_without_runtime(
        self,
        root_override: Option<&std::path::Path>,
    ) -> Result<(), OrbitError> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let roots = OrbitRuntime::resolve_roots_for_cwd(&cwd, root_override)?;
        let report = migrate_learning_layout_at(&roots.shared_root)?;
        if !report.already_migrated {
            let runtime = OrbitRuntime::from_resolved_roots(
                &roots.global_root,
                &roots.shared_root,
                &roots.local_root,
            )?;
            runtime.sync_learnings()?;
        }
        print_report(&report, self.json)
    }
}

impl Execute for LearningMigrateLayoutArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let report = runtime.migrate_learning_layout()?;
        if !report.already_migrated {
            runtime.sync_learnings()?;
        }
        print_report(&report, self.json)
    }
}

fn print_report(
    report: &orbit_core::LearningLayoutMigrationReport,
    json_output: bool,
) -> Result<(), OrbitError> {
    if json_output {
        return crate::output::json::print_pretty(&json!({
            "already_migrated": report.already_migrated,
            "moved_active": report.moved_active,
            "moved_superseded": report.moved_superseded,
            "moved_total": report.moved_total(),
            "removed_superseded_dir": report.removed_superseded_dir,
        }));
    }

    if report.already_migrated {
        println!("workspace is already on the per-entity layout");
    } else {
        println!(
            "Migrated learning layout: moved {} active, {} superseded; removed superseded directory: {}",
            report.moved_active, report.moved_superseded, report.removed_superseded_dir
        );
    }
    Ok(())
}
