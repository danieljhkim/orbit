use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

use super::output::learning_comment_to_json;

#[derive(Args)]
pub struct LearningCommentCommand {
    #[command(subcommand)]
    pub command: LearningCommentSubcommand,
}

impl Execute for LearningCommentCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum LearningCommentSubcommand {
    /// Add a footnote-style comment to an active learning
    Add(LearningCommentAddArgs),
    /// List comments for one learning
    List(LearningCommentListArgs),
    /// Soft-delete a comment by appending a tombstone
    Delete(LearningCommentDeleteArgs),
}

impl Execute for LearningCommentSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            LearningCommentSubcommand::Add(args) => args.execute(runtime),
            LearningCommentSubcommand::List(args) => args.execute(runtime),
            LearningCommentSubcommand::Delete(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct LearningCommentAddArgs {
    /// Parent learning ID
    #[arg(long = "learning-id")]
    pub learning_id: String,
    /// Comment body (trimmed, non-empty, ≤ 500 chars)
    #[arg(long)]
    pub body: String,
    /// Author model or canonical agent family
    #[arg(long)]
    pub model: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningCommentAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let comment = runtime.add_learning_comment(self.learning_id, self.body, self.model)?;
        if self.json {
            crate::output::json::print_pretty(&json!({
                "id": comment.id,
                "learning_id": comment.learning_id,
                "created_at": comment.created_at.to_rfc3339(),
            }))
        } else {
            println!("{}", comment.id);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct LearningCommentListArgs {
    /// Parent learning ID
    #[arg(long = "learning-id")]
    pub learning_id: String,
    /// Include comments that have a delete tombstone
    #[arg(long = "include-deleted")]
    pub include_deleted: bool,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningCommentListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let comments = runtime.list_learning_comments(&self.learning_id, self.include_deleted)?;
        if self.json {
            let array = Value::Array(comments.iter().map(learning_comment_to_json).collect());
            crate::output::json::print_pretty(&array)
        } else {
            for comment in &comments {
                println!("{}\t{}\t{}", comment.id, comment.learning_id, comment.body);
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct LearningCommentDeleteArgs {
    /// Comment ID
    #[arg(long = "id")]
    pub id: String,
    /// Deleting model or canonical agent family
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningCommentDeleteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.delete_learning_comment(self.id.clone(), self.model)?;
        if self.json {
            crate::output::json::print_pretty(&json!({
                "id": self.id,
                "deleted": true,
            }))
        } else {
            println!("{}", self.id);
            Ok(())
        }
    }
}
