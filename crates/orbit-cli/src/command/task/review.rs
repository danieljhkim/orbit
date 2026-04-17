use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Manage review threads on a task")]
pub struct ReviewThreadCommand {
    #[command(subcommand)]
    pub command: ReviewThreadSubcommand,
}

impl Execute for ReviewThreadCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum ReviewThreadSubcommand {
    /// Create a new review thread on a task
    Add(ReviewThreadAddArgs),
    /// List review threads on a task
    List(ReviewThreadListArgs),
    /// Reply to an existing review thread
    Reply(ReviewThreadReplyArgs),
    /// Resolve a review thread
    Resolve(ReviewThreadResolveArgs),
}

impl Execute for ReviewThreadSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ReviewThreadSubcommand::Add(args) => args.execute(runtime),
            ReviewThreadSubcommand::List(args) => args.execute(runtime),
            ReviewThreadSubcommand::Reply(args) => args.execute(runtime),
            ReviewThreadSubcommand::Resolve(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct ReviewThreadAddArgs {
    /// Task ID
    pub id: String,
    /// Review comment body
    #[arg(long)]
    pub body: String,
    /// File path for inline comment
    #[arg(long)]
    pub path: Option<String>,
    /// Line number for inline comment
    #[arg(long)]
    pub line: Option<u64>,
    /// Explicit agent name
    #[arg(long)]
    pub agent: Option<String>,
    /// Explicit agent model
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for ReviewThreadAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let thread = runtime.add_review_thread(
            &self.id, self.body, self.path, self.line, self.agent, self.model,
        )?;
        if self.json {
            crate::output::json::print_pretty(&serde_json::to_value(&thread).unwrap_or_default())
        } else {
            println!("Created review thread '{}'", thread.thread_id);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct ReviewThreadListArgs {
    /// Task ID
    pub id: String,
    /// Filter by thread status (open, resolved)
    #[arg(long)]
    pub status: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for ReviewThreadListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let status_filter = self
            .status
            .map(|s| {
                s.parse::<orbit_core::ReviewThreadStatus>()
                    .map_err(OrbitError::InvalidInput)
            })
            .transpose()?;
        let threads = runtime.list_review_threads(&self.id, status_filter)?;
        if self.json {
            crate::output::json::print_pretty(&serde_json::to_value(&threads).unwrap_or_default())
        } else {
            for t in &threads {
                println!(
                    "{}\t{}\t{} message(s)",
                    t.thread_id,
                    t.status,
                    t.messages.len()
                );
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct ReviewThreadReplyArgs {
    /// Task ID
    pub id: String,
    /// Thread ID to reply to
    pub thread_id: String,
    /// Reply body
    #[arg(long)]
    pub body: String,
    /// Explicit agent name
    #[arg(long)]
    pub agent: Option<String>,
    /// Explicit agent model
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for ReviewThreadReplyArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let thread = runtime.reply_review_thread(
            &self.id,
            &self.thread_id,
            self.body,
            self.agent,
            self.model,
        )?;
        if self.json {
            crate::output::json::print_pretty(&serde_json::to_value(&thread).unwrap_or_default())
        } else {
            println!(
                "Replied to thread '{}' ({} messages)",
                thread.thread_id,
                thread.messages.len()
            );
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct ReviewThreadResolveArgs {
    /// Task ID
    pub id: String,
    /// Thread ID to resolve
    pub thread_id: String,
    /// Explicit agent name
    #[arg(long)]
    pub agent: Option<String>,
    /// Explicit agent model
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for ReviewThreadResolveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let thread =
            runtime.resolve_review_thread(&self.id, &self.thread_id, self.agent, self.model)?;
        if self.json {
            crate::output::json::print_pretty(&serde_json::to_value(&thread).unwrap_or_default())
        } else {
            println!("Resolved thread '{}'", thread.thread_id);
            Ok(())
        }
    }
}
