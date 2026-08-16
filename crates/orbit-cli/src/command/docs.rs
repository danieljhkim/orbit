use std::str::FromStr;

use clap::{Args, Subcommand};
use orbit_core::application::semantic::{IndexKind, SemanticIndexParams, SemanticIndexResult};
use orbit_core::{DocType, OrbitError, OrbitRuntime};
use serde::Serialize;
use serde_json::Value;

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

#[derive(Args)]
#[command(about = "List, show, and curate the indexed docs corpus")]
pub struct DocsCommand {
    #[command(subcommand)]
    pub command: DocsSubcommand,
}

#[derive(Subcommand)]
pub enum DocsSubcommand {
    /// List indexed Markdown docs under configured roots
    List(DocsListArgs),
    /// Show one doc with parsed frontmatter and body
    Show(DocsShowArgs),
    /// Register an additional docs root in .orbit/config.toml
    Add(DocsAddArgs),
    // ADR-0180: docs embeddings are built by an explicit admin verb; the old no-op reindex verb is retired.
    /// Build or refresh doc corpus embeddings
    Index(DocsIndexArgs),
    /// Backfill locked docs frontmatter for legacy docs
    Migrate(DocsMigrateArgs),
}

#[derive(Args)]
pub struct DocsListArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
    /// Filter by doc type (design | pattern | context | glossary | runbook)
    #[arg(long = "type")]
    pub doc_type: Option<String>,
    /// Filter by tag
    #[arg(long)]
    pub tag: Option<String>,
}

#[derive(Args)]
pub struct DocsShowArgs {
    /// Repo-relative path to a Markdown doc
    pub path: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct DocsAddArgs {
    /// Existing path to add to [docs].roots
    pub path: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct DocsIndexArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
    /// Re-embed even when content hashes are unchanged
    #[arg(long)]
    pub force: bool,
    /// Embedding model alias to use for indexing
    #[arg(long)]
    pub model: Option<String>,
}

#[derive(Args)]
pub struct DocsMigrateArgs {
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

impl Execute for DocsCommand {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        match self.command {
            DocsSubcommand::List(args) => args.execute(runtime),
            DocsSubcommand::Show(args) => args.execute(runtime),
            DocsSubcommand::Add(args) => args.execute(runtime),
            DocsSubcommand::Index(args) => args.execute(runtime),
            DocsSubcommand::Migrate(args) => args.execute(runtime),
        }
    }
}

impl Execute for DocsListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let doc_type = self
            .doc_type
            .as_deref()
            .map(DocType::from_str)
            .transpose()
            .map_err(OrbitError::InvalidInput)?;
        let records = runtime.list_docs(doc_type, self.tag.as_deref())?;
        let values = to_json_values(&records)?;

        {
            use crate::output::table::{Column, Table};
            // `orbit docs show <path>` prints a doc in full.
            let mut table = Table::new(vec![
                Column::new("PATH").path(),
                Column::new("TYPE").fixed().filtered(doc_type.is_some()),
                Column::new("SUMMARY"),
                Column::new("TAGS").filtered(self.tag.is_some()),
                Column::new("RELATED"),
            ])
            .empty_message("no docs matching the given filters");
            for record in records {
                table.add_row(vec![
                    record.path,
                    record.frontmatter.doc_type.to_string(),
                    record.frontmatter.summary,
                    record.frontmatter.tags.join(", "),
                    record
                        .frontmatter
                        .related_artifacts
                        .iter()
                        .map(|artifact| artifact.as_str().to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                ]);
            }
            Ok(Payload::list(values, table).into())
        }
    }
}

impl Execute for DocsShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let shown = runtime.show_doc(&self.path)?;
        if self.json {
            {
                print_json(&shown)?;
                Ok(CommandOutput::Silent)
            }
        } else {
            println!("Path: {}", shown.path);
            println!("Type: {}", shown.frontmatter.doc_type);
            println!("Summary: {}", shown.frontmatter.summary);
            if !shown.frontmatter.tags.is_empty() {
                println!("Tags: {}", shown.frontmatter.tags.join(", "));
            }
            if !shown.frontmatter.paths.is_empty() {
                println!("Paths: {}", shown.frontmatter.paths.join(", "));
            }
            if !shown.frontmatter.related_features.is_empty() {
                println!(
                    "Related Features: {}",
                    shown.frontmatter.related_features.join(", ")
                );
            }
            if !shown.frontmatter.related_artifacts.is_empty() {
                println!(
                    "Related Artifacts: {}",
                    shown
                        .frontmatter
                        .related_artifacts
                        .iter()
                        .map(|artifact| artifact.as_str().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            println!("\n{}", shown.body);
            Ok(CommandOutput::Silent)
        }
    }
}

impl Execute for DocsAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let outcome = runtime.add_docs_root(&self.path)?;
        if self.json {
            {
                print_json(&outcome)?;
                Ok(CommandOutput::Silent)
            }
        } else if outcome.added {
            println!("Added docs root: {}", outcome.path);
            Ok(CommandOutput::Silent)
        } else {
            println!("Docs root already registered: {}", outcome.path);
            Ok(CommandOutput::Silent)
        }
    }
}

impl Execute for DocsIndexArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let result = runtime.semantic_index(SemanticIndexParams {
            model: self.model,
            force: self.force,
            kind: Some(IndexKind::Docs),
        })?;
        if self.json {
            Ok(Payload::document(serde_json::json!(result)).into())
        } else {
            {
                print_docs_index_text(result)?;
                Ok(CommandOutput::Silent)
            }
        }
    }
}

fn print_docs_index_text(result: SemanticIndexResult) -> Result<(), OrbitError> {
    let SemanticIndexResult::Docs {
        model_id,
        report,
        indexed_sources,
        stale_sources,
    } = result
    else {
        return Err(OrbitError::Execution(
            "docs index alias returned a non-docs report".to_string(),
        ));
    };
    println!(
        "Indexed docs: model={} indexed_sources={} embedded_chunks={} skipped_fields={} stale_sources={}",
        model_id,
        indexed_sources,
        report.embedded_chunks,
        report.skipped_fields,
        stale_sources.len()
    );
    Ok(())
}

impl Execute for DocsMigrateArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let report = runtime.migrate_docs(!self.confirm)?;
        if self.json {
            {
                print_json(&report)?;
                Ok(CommandOutput::Silent)
            }
        } else if report.changed.is_empty() {
            println!("No docs need migration.");
            Ok(CommandOutput::Silent)
        } else {
            for change in &report.changed {
                println!("{}", change.diff);
            }
            if report.dry_run {
                println!("{} doc(s) would be migrated.", report.changed.len());
            } else {
                println!("Migrated {} doc(s).", report.changed.len());
            }
            Ok(CommandOutput::Silent)
        }
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<(), OrbitError> {
    let value: Value = serde_json::to_value(value)
        .map_err(|error| OrbitError::Execution(format!("serialize docs output: {error}")))?;
    crate::output::json::print_pretty(&value)
}

/// The list's records, in the same shape `--json` has always emitted.
fn to_json_values<T: Serialize>(records: &[T]) -> Result<Vec<Value>, OrbitError> {
    records
        .iter()
        .map(|record| {
            serde_json::to_value(record)
                .map_err(|error| OrbitError::Execution(format!("serialize docs output: {error}")))
        })
        .collect()
}
