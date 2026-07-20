//! Human-only global allocation and local-derived knowledge maintenance.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use orbit_common::types::KnowledgeIdKind;
use orbit_core::{OrbitError, runtime::resolve_global_root};
use orbit_remote::{allocate_knowledge_id_for_human, validate_local_knowledge_for_sync};
use serde_json::json;

use super::operation::DispatchContext;

#[derive(Args)]
#[command(about = "Allocate global knowledge IDs and rebuild checkout-local indexes")]
pub struct KnowledgeCommand {
    #[command(subcommand)]
    pub command: KnowledgeSubcommand,
}

#[derive(Subcommand)]
pub enum KnowledgeSubcommand {
    /// Allocate one hub-global ID without authoring a record
    Allocate(KnowledgeAllocateArgs),
    /// Validate committed files and rebuild checkout-local indexes
    Sync(KnowledgeSyncArgs),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AllocateKind {
    Adr,
    Learning,
}

impl From<AllocateKind> for KnowledgeIdKind {
    fn from(value: AllocateKind) -> Self {
        match value {
            AllocateKind::Adr => Self::Adr,
            AllocateKind::Learning => Self::Learning,
        }
    }
}

#[derive(Args)]
pub struct KnowledgeAllocateArgs {
    #[arg(long, value_enum)]
    pub kind: AllocateKind,
    /// Stable logical workspace ID or an absolute registered local path
    #[arg(long)]
    pub workspace: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SyncKind {
    Adr,
    Learning,
    All,
}

#[derive(Args)]
pub struct KnowledgeSyncArgs {
    #[arg(long, value_enum, default_value = "all")]
    pub kind: SyncKind,
    /// Exact registered local checkout path
    #[arg(long)]
    pub workspace: PathBuf,
    #[arg(long)]
    pub json: bool,
}

impl KnowledgeCommand {
    pub fn execute_without_runtime(self, _context: DispatchContext<'_>) -> Result<(), OrbitError> {
        match self.command {
            KnowledgeSubcommand::Allocate(args) => args.execute(),
            KnowledgeSubcommand::Sync(args) => args.execute(),
        }
    }
}

impl KnowledgeAllocateArgs {
    fn execute(self) -> Result<(), OrbitError> {
        let allocation = allocate_knowledge_id_for_human(
            &self.workspace,
            self.kind.into(),
            Some("human".into()),
        )?;
        let value = json!({
            "kind": allocation.kind.as_str(),
            "workspace_id": allocation.workspace_id,
            "id": allocation.id,
            "mcp_call_id": allocation.mcp_call_id,
        });
        if self.json {
            crate::output::json::print_pretty(&value)
        } else {
            println!(
                "{} {} (workspace {}, mcp_call_id {})",
                value["kind"].as_str().unwrap_or_default(),
                value["id"].as_str().unwrap_or_default(),
                value["workspace_id"].as_str().unwrap_or_default(),
                value["mcp_call_id"].as_str().unwrap_or_default(),
            );
            Ok(())
        }
    }
}

impl KnowledgeSyncArgs {
    fn execute(self) -> Result<(), OrbitError> {
        let selected = exact_checkout(&self.workspace)?;
        let include_adrs = matches!(self.kind, SyncKind::Adr | SyncKind::All);
        let include_learnings = matches!(self.kind, SyncKind::Learning | SyncKind::All);
        let global_root = resolve_global_root()?;

        // Validate the entire selected corpus before opening a mutating index
        // rebuild. Invalid or unavailable allocation state leaves both indexes
        // untouched, including for --kind all.
        let counts = validate_local_knowledge_for_sync(
            &global_root,
            &selected,
            include_adrs,
            include_learnings,
        )?;
        let roots =
            orbit_remote::runtime::RemoteRuntimeFactory::resolve_roots_for_cwd(&selected, None)?;
        let runtime = orbit_remote::runtime::RemoteRuntimeFactory::open_resolved_roots(roots)?;
        let runtime_repo = runtime.paths().repo_root.canonicalize().map_err(|error| {
            OrbitError::InvalidInput(format!(
                "knowledge sync resolved checkout '{}' is unavailable: {error}",
                runtime.paths().repo_root.display()
            ))
        })?;
        if runtime_repo != selected {
            return Err(OrbitError::InvalidInput(format!(
                "knowledge sync requires one exact checkout: requested '{}', resolved '{}'",
                selected.display(),
                runtime_repo.display()
            )));
        }
        runtime.sync_knowledge_indexes(include_adrs, include_learnings)?;
        let value = json!({
            "workspace": selected,
            "kind": match self.kind { SyncKind::Adr => "adr", SyncKind::Learning => "learning", SyncKind::All => "all" },
            "adr_rebuilt_count": counts.adrs,
            "learning_rebuilt_count": counts.learnings,
            "rebuilt_count": counts.adrs + counts.learnings,
        });
        if self.json {
            crate::output::json::print_pretty(&value)
        } else {
            println!(
                "Synced {} knowledge records ({} ADRs, {} learnings)",
                counts.adrs + counts.learnings,
                counts.adrs,
                counts.learnings
            );
            Ok(())
        }
    }
}

fn exact_checkout(path: &Path) -> Result<PathBuf, OrbitError> {
    if !path.is_absolute() {
        return Err(OrbitError::InvalidInput(format!(
            "knowledge sync --workspace must be an absolute exact checkout path, got '{}'",
            path.display()
        )));
    }
    path.canonicalize().map_err(|error| {
        OrbitError::InvalidInput(format!(
            "knowledge sync checkout '{}' is unavailable: {error}",
            path.display()
        ))
    })
}
