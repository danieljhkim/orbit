use clap::{Args, Subcommand, ValueEnum};
use orbit_core::command::semantic::{
    IndexKind, SemanticIndexParams, SemanticIndexResult, SemanticInstallParams,
    SemanticUninstallParams,
};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Manage local orbit-search indexing")]
pub struct SemanticCommand {
    #[command(subcommand)]
    pub command: SemanticSubcommand,
}

#[derive(Subcommand)]
pub enum SemanticSubcommand {
    /// Download the search companion and selected model
    Install(SemanticInstallArgs),
    /// Remove installed orbit-search companion and/or models
    Uninstall(SemanticUninstallArgs),
    /// Show orbit-search index and companion status
    Stats(SemanticStatsArgs),
    /// Rebuild semantic embeddings
    Index(SemanticIndexArgs),
}

#[derive(Args)]
pub struct SemanticInstallArgs {
    #[arg(long)]
    pub model: Option<String>,
    /// Replace the companion even when the installed version is current
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct SemanticUninstallArgs {
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct SemanticIndexArgs {
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub force: bool,
    #[arg(
        long,
        value_enum,
        default_value_t = SemanticIndexKindArg::Tasks,
        value_name = "KIND",
        help = "--kind selects corpus: tasks (default), docs (same as `orbit docs index`), all (rebuilds both)."
    )]
    pub kind: SemanticIndexKindArg,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SemanticIndexKindArg {
    Tasks,
    Docs,
    All,
}

impl From<SemanticIndexKindArg> for IndexKind {
    fn from(value: SemanticIndexKindArg) -> Self {
        match value {
            SemanticIndexKindArg::Tasks => Self::Tasks,
            SemanticIndexKindArg::Docs => Self::Docs,
            SemanticIndexKindArg::All => Self::All,
        }
    }
}

#[derive(Args)]
pub struct SemanticStatsArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for SemanticCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

impl Execute for SemanticSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            SemanticSubcommand::Install(args) => args.execute(runtime),
            SemanticSubcommand::Uninstall(args) => args.execute(runtime),
            SemanticSubcommand::Stats(args) => args.execute(runtime),
            SemanticSubcommand::Index(args) => args.execute(runtime),
        }
    }
}

impl Execute for SemanticInstallArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let result = runtime.semantic_install(SemanticInstallParams {
            model: self.model,
            force: self.force,
        })?;
        if self.json {
            crate::output::json::print_pretty(&json!(result))
        } else {
            println!(
                "Installed semantic search: companion={} model={} companion_changed={} model_changed={}",
                result.companion_path,
                result.model_id,
                result.companion_changed,
                result.model_installed
            );
            Ok(())
        }
    }
}

impl Execute for SemanticUninstallArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let result = runtime.semantic_uninstall(SemanticUninstallParams {
            model: self.model,
            all: self.all,
        })?;
        if self.json {
            crate::output::json::print_pretty(&json!(result))
        } else {
            println!(
                "Removed semantic search assets: companion={} models={}",
                result.removed_companion,
                if result.removed_models.is_empty() {
                    "-".to_string()
                } else {
                    result.removed_models.join(", ")
                }
            );
            Ok(())
        }
    }
}

impl Execute for SemanticIndexArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let result = runtime.semantic_index(SemanticIndexParams {
            model: self.model,
            force: self.force,
            kind: Some(self.kind.into()),
        })?;
        if self.json {
            crate::output::json::print_pretty(&json!(result))
        } else {
            print_semantic_index_text(result)
        }
    }
}

fn print_semantic_index_text(result: SemanticIndexResult) -> Result<(), OrbitError> {
    match result {
        SemanticIndexResult::Tasks { model_id, report } => {
            println!(
                "Indexed semantic search: model={} embedded_chunks={} skipped_fields={}",
                model_id, report.embedded_chunks, report.skipped_fields
            );
        }
        SemanticIndexResult::Docs {
            model_id,
            report,
            indexed_sources,
            stale_sources,
        } => {
            println!(
                "Indexed docs: model={} indexed_sources={} embedded_chunks={} skipped_fields={} stale_sources={}",
                model_id,
                indexed_sources,
                report.embedded_chunks,
                report.skipped_fields,
                stale_sources.len()
            );
        }
        SemanticIndexResult::All { tasks, docs } => {
            println!(
                "Indexed semantic search: tasks_model={} tasks_embedded_chunks={} tasks_skipped_fields={} docs_model={} docs_indexed_sources={} docs_embedded_chunks={} docs_skipped_fields={} docs_stale_sources={}",
                tasks.model_id,
                tasks.report.embedded_chunks,
                tasks.report.skipped_fields,
                docs.model_id,
                docs.indexed_sources,
                docs.report.embedded_chunks,
                docs.report.skipped_fields,
                docs.stale_sources.len()
            );
        }
    }
    Ok(())
}

impl Execute for SemanticStatsArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let result = runtime.semantic_stats()?;
        if self.json {
            crate::output::json::print_pretty(&json!(result))
        } else {
            let mut table = crate::output::table::build_table(&["SOURCE_KIND", "MODEL", "ROWS"]);
            for row in &result.rows.counts {
                table.add_row(vec![
                    row.source_kind.clone(),
                    row.model_id.clone(),
                    row.rows.to_string(),
                ]);
            }
            println!("{table}");
            println!(
                "stale_rows={} companion={} version={} active_model={}",
                result.rows.stale_rows,
                if result.companion.installed {
                    "installed"
                } else {
                    "not_installed"
                },
                result.companion.version.as_deref().unwrap_or("-"),
                result.companion.model.as_deref().unwrap_or("-")
            );
            Ok(())
        }
    }
}
