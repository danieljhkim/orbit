use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::add::LearningAddArgs;
use super::comment::LearningCommentCommand;
use super::list::LearningListArgs;
use super::migrate_layout::LearningMigrateLayoutArgs;
use super::prune::LearningPruneArgs;
use super::reindex::LearningReindexArgs;
use super::search::LearningSearchArgs;
use super::show::LearningShowArgs;
use super::supersede::LearningSupersedeArgs;
use super::update::LearningUpdateArgs;
use super::upvote::LearningUpvoteArgs;

#[derive(Args)]
#[command(about = "Create, search, and curate project learnings")]
pub struct LearningCommand {
    #[command(subcommand)]
    pub command: LearningSubcommand,
}

impl Execute for LearningCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum LearningSubcommand {
    /// Create a new active learning
    Add(LearningAddArgs),
    /// Add, list, or delete comments anchored to a learning
    Comment(LearningCommentCommand),
    /// List learnings filtered by status, tag, or path
    List(LearningListArgs),
    /// Search active learnings by path glob OR tag OR substring
    Search(LearningSearchArgs),
    /// Show a single learning by ID
    Show(LearningShowArgs),
    /// Update an existing active learning
    Update(LearningUpdateArgs),
    /// Record a task-anchored upvote for a learning
    Upvote(LearningUpvoteArgs),
    /// Mark a learning as superseded by another
    Supersede(LearningSupersedeArgs),
    /// Rebuild the SQLite envelope index from YAML
    Reindex(LearningReindexArgs),
    /// Migrate legacy flat learning YAML files to per-entity directories
    MigrateLayout(LearningMigrateLayoutArgs),
    /// Report or archive stale learnings
    Prune(LearningPruneArgs),
}

impl Execute for LearningSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            LearningSubcommand::Add(args) => args.execute(runtime),
            LearningSubcommand::Comment(args) => args.execute(runtime),
            LearningSubcommand::List(args) => args.execute(runtime),
            LearningSubcommand::Search(args) => args.execute(runtime),
            LearningSubcommand::Show(args) => args.execute(runtime),
            LearningSubcommand::Update(args) => args.execute(runtime),
            LearningSubcommand::Upvote(args) => args.execute(runtime),
            LearningSubcommand::Supersede(args) => args.execute(runtime),
            LearningSubcommand::Reindex(args) => args.execute(runtime),
            LearningSubcommand::MigrateLayout(args) => args.execute(runtime),
            LearningSubcommand::Prune(args) => args.execute(runtime),
        }
    }
}
