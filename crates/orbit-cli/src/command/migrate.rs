use clap::Args;
use orbit_cmd::{MigrateCommands, MigrateStatus, migrate_dry_run_at};
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_remote::runtime::RemoteRuntimeFactory;
use serde_json::json;

use crate::command::Execute;

/// `orbit migrate` — versioned `.orbit/` upgrade surface [ORB-10012].
///
/// Both migration ledgers auto-apply on workspace open (the SQLite schema
/// ledger inside `Store::open`, the workspace-layout registry in the runtime
/// pre-flight), so the apply path simply opens the runtime and reports what
/// happened. The default and `--dry-run` inspect *without* opening a runtime,
/// which is the only way to list pending migrations instead of silently
/// applying them. `--confirm` selects the apply path.
#[derive(Args)]
#[command(about = "Apply or inspect pending .orbit layout and store schema migrations")]
pub struct MigrateCommand {
    /// Apply pending migrations
    #[arg(long, conflicts_with = "dry_run")]
    pub confirm: bool,
    /// Explicitly request the default non-destructive inspection mode
    #[arg(long, conflicts_with = "confirm")]
    pub dry_run: bool,
    /// Emit machine-readable JSON instead of the table.
    #[arg(long)]
    pub json: bool,
}

impl MigrateCommand {
    /// `--dry-run` dispatches before the runtime bootstrap in `main.rs`:
    /// opening a runtime would auto-apply the very migrations being listed.
    pub fn execute_without_runtime(
        self,
        root_override: Option<&std::path::Path>,
    ) -> Result<(), OrbitError> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let Some(resolved) =
            RemoteRuntimeFactory::try_resolve_initialized_roots(&cwd, root_override)?
        else {
            return Err(OrbitError::WorkspaceError(
                "no initialized orbit workspace found from the current directory; run `orbit init` first"
                    .to_string(),
            ));
        };
        let global_root = orbit_remote::workspace_registry::global_orbit_dir()?;
        let status = migrate_dry_run_at(&global_root, &resolved.shared_root)?;
        print_status(&status, true, self.json)?;

        if status.newer_than_binary() {
            return Err(OrbitError::Migration(format!(
                "workspace '{}' was written by a newer orbit than this binary supports; \
                 upgrade orbit",
                status.orbit_dir.display()
            )));
        }
        let pending = status.pending_total();
        if pending > 0 {
            return Err(OrbitError::Execution(format!(
                "{pending} migration(s) pending; run `orbit migrate --confirm` to apply"
            )));
        }
        Ok(())
    }
}

impl Execute for MigrateCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        // Apply path: opening the runtime (before this command ran) already
        // applied everything pending; report the outcome.
        let status = runtime.migrate_status()?;
        print_status(&status, false, self.json)
    }
}

fn print_status(
    status: &MigrateStatus,
    dry_run: bool,
    json_output: bool,
) -> Result<(), OrbitError> {
    if json_output {
        return crate::output::json::print_pretty(&json!({
            "orbit_dir": status.orbit_dir.to_string_lossy(),
            "binary_version": status.binary_version,
            "dry_run": dry_run,
            "layout": {
                "current": status.layout_version,
                "supported": status.layout_supported,
                "pending": status.pending_layout.iter().map(|m| json!({
                    "version": m.version,
                    "name": m.name,
                    "description": m.description,
                })).collect::<Vec<_>>(),
            },
            "schema": {
                "current": status.schema_version,
                "supported": status.schema_supported,
                "pending": status.pending_schema.iter().map(|m| json!({
                    "version": m.version,
                    "name": m.name,
                })).collect::<Vec<_>>(),
            },
            "applied_layout": status.applied_layout.iter().map(|m| json!({
                "version": m.version,
                "name": m.name,
            })).collect::<Vec<_>>(),
            "up_to_date": status.pending_total() == 0 && !status.newer_than_binary(),
        }));
    }

    // A fixed-shape status readout, not a result set: every component is named
    // even when the two version columns happen to agree.
    let mut table = crate::output::table::build_table(&["COMPONENT", "CURRENT", "SUPPORTED"])
        .keep_all_columns();
    table.add_row(vec![
        "workspace layout".to_string(),
        status.layout_version.to_string(),
        status.layout_supported.to_string(),
    ]);
    table.add_row(vec![
        "store schema".to_string(),
        status.schema_version.to_string(),
        status.schema_supported.to_string(),
    ]);
    table.print();
    println!(
        "\nworkspace: {} (orbit {})",
        status.orbit_dir.display(),
        status.binary_version
    );

    for applied in &status.applied_layout {
        println!(
            "applied on open: layout v{} ({})",
            applied.version, applied.name
        );
    }

    if status.newer_than_binary() {
        return Ok(()); // the caller emits the refusal error
    }
    if status.pending_total() == 0 {
        println!(
            "\n{}",
            crate::output::color::job_state_color("Workspace is up to date.")
        );
        return Ok(());
    }

    println!("\nPending migrations:");
    for pending in &status.pending_layout {
        println!(
            "  layout v{} ({}) — {}",
            pending.version, pending.name, pending.description
        );
    }
    for pending in &status.pending_schema {
        println!("  schema v{} ({})", pending.version, pending.name);
    }
    println!(
        "\nRun `orbit migrate --confirm` to apply (migrations also auto-apply on workspace open).\n\
         Before a major upgrade, consider backing up workspace state first:\n\
         cp -a {orbit_dir} {orbit_dir}.bak",
        orbit_dir = status.orbit_dir.display()
    );
    Ok(())
}
