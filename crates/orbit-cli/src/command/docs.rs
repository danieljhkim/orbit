use std::str::FromStr;

use clap::{Args, Subcommand};
use orbit_core::command::docs::DocIndexParams;
use orbit_core::{DocType, OrbitError, OrbitRuntime};
use serde::Serialize;
use serde_json::Value;

use crate::command::Execute;

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
    /// Print planned diffs without writing files
    #[arg(long)]
    pub dry_run: bool,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for DocsCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
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
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let doc_type = self
            .doc_type
            .as_deref()
            .map(DocType::from_str)
            .transpose()
            .map_err(OrbitError::InvalidInput)?;
        let records = runtime.list_docs(doc_type, self.tag.as_deref())?;
        if self.json {
            print_json(&records)
        } else {
            let mut table =
                crate::output::table::build_table(&["PATH", "TYPE", "SUMMARY", "TAGS", "RELATED"]);
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
            println!("{table}");
            Ok(())
        }
    }
}

impl Execute for DocsShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let shown = runtime.show_doc(&self.path)?;
        if self.json {
            print_json(&shown)
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
            Ok(())
        }
    }
}

impl Execute for DocsAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let outcome = runtime.add_docs_root(&self.path)?;
        if self.json {
            print_json(&outcome)
        } else if outcome.added {
            println!("Added docs root: {}", outcome.path);
            Ok(())
        } else {
            println!("Docs root already registered: {}", outcome.path);
            Ok(())
        }
    }
}

impl Execute for DocsIndexArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let result = runtime.index_docs(DocIndexParams {
            model: self.model,
            force: self.force,
        })?;
        if self.json {
            crate::output::json::print_pretty(&serde_json::json!(result))
        } else {
            println!(
                "Indexed docs: model={} indexed_sources={} embedded_chunks={} skipped_fields={} stale_sources={}",
                result.model_id,
                result.indexed_sources,
                result.report.embedded_chunks,
                result.report.skipped_fields,
                result.stale_sources.len()
            );
            Ok(())
        }
    }
}

impl Execute for DocsMigrateArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let report = runtime.migrate_docs(self.dry_run)?;
        if self.json {
            print_json(&report)
        } else if report.changed.is_empty() {
            println!("No docs need migration.");
            Ok(())
        } else {
            for change in &report.changed {
                println!("{}", change.diff);
            }
            if report.dry_run {
                println!("{} doc(s) would be migrated.", report.changed.len());
            } else {
                println!("Migrated {} doc(s).", report.changed.len());
            }
            Ok(())
        }
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<(), OrbitError> {
    let value: Value = serde_json::to_value(value)
        .map_err(|error| OrbitError::Execution(format!("serialize docs output: {error}")))?;
    crate::output::json::print_pretty(&value)
}
