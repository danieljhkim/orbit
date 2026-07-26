use clap::Args;
use orbit_core::{
    OrbitError, OrbitRuntime, inspect_learning_layout_at, migrate_learning_layout_at,
};
use orbit_remote::runtime::RemoteRuntimeFactory;
use serde_json::json;

use crate::command::Execute;

#[derive(Args)]
pub struct LearningMigrateLayoutArgs {
    /// Apply the migration; without this flag the command only reports
    #[arg(long, conflicts_with = "dry_run")]
    pub confirm: bool,
    /// Explicitly request the default non-destructive mode
    #[arg(long, conflicts_with = "confirm")]
    pub dry_run: bool,
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
        let roots = RemoteRuntimeFactory::resolve_roots_for_cwd(&cwd, root_override)?;
        let dry_run = !self.confirm;
        let report = if dry_run {
            inspect_learning_layout_at(&roots.shared_root)?
        } else {
            migrate_learning_layout_at(&roots.shared_root)?
        };
        if !dry_run && !report.already_migrated {
            let runtime = RemoteRuntimeFactory::open_resolved_roots(roots)?;
            runtime.sync_learnings()?;
        }
        print_report(&report, dry_run, self.json)
    }
}

impl Execute for LearningMigrateLayoutArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let dry_run = !self.confirm;
        let report = if dry_run {
            runtime.inspect_learning_layout()?
        } else {
            runtime.migrate_learning_layout()?
        };
        if !dry_run && !report.already_migrated {
            runtime.sync_learnings()?;
        }
        print_report(&report, dry_run, self.json)
    }
}

fn print_report(
    report: &orbit_core::LearningLayoutMigrationReport,
    dry_run: bool,
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
    } else if dry_run {
        println!(
            "Would migrate learning layout: move {} active, {} superseded; remove superseded directory: {}",
            report.moved_active, report.moved_superseded, report.removed_superseded_dir
        );
    } else {
        println!(
            "Migrated learning layout: moved {} active, {} superseded; removed superseded directory: {}",
            report.moved_active, report.moved_superseded, report.removed_superseded_dir
        );
    }
    Ok(())
}
