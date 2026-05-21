use clap::{ArgAction, ArgGroup, Args, ValueEnum};
use orbit_core::{GlobalSearchHit, GlobalSearchKind, GlobalSearchParams, OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
#[command(
    about = "Search tasks, docs, learnings, and ADRs",
    after_help = "Index coverage note: vector search runs against tasks only today; docs, learnings, and ADRs use lexical matching regardless of --hybrid."
)]
#[command(group(
    ArgGroup::new("query_mode")
        .args(["query", "semantic"])
        .multiple(false)
))]
#[command(group(
    ArgGroup::new("search_input")
        .args(["query", "semantic", "path", "tags"])
        .required(true)
        .multiple(true)
))]
pub struct SearchCommand {
    /// Free-text query. Defaults to lexical matching unless --hybrid is set.
    #[arg(value_name = "query")]
    pub query: Option<String>,
    // ADR-0175: `--hybrid` names the free-text BM25 + cosine ranker.
    /// Use hybrid BM25 + cosine ranking for indexed task fields. Other kinds remain lexical.
    #[arg(long)]
    pub hybrid: bool,
    // ADR-0175: `--semantic <id>` names task-neighbor lookup.
    /// Find cosine-neighbor tasks for a known task ID. Requires task vectors.
    #[arg(long, value_name = "id")]
    pub semantic: Option<String>,
    /// Restrict results to one corpus kind.
    #[arg(long, value_enum, default_value_t = SearchKindArg::All)]
    pub kind: SearchKindArg,
    /// Maximum number of results to return.
    #[arg(long, default_value_t = 10)]
    pub limit: usize,
    /// Optional indexed task field filter for semantic task search.
    #[arg(long)]
    pub field: Option<String>,
    /// Optional semantic embedding model alias, such as bge-small.
    #[arg(long)]
    pub model: Option<String>,
    /// Filter by tag (AND semantics). Applies to task, doc, learning, and ADR.
    #[arg(long = "tag", action = ArgAction::Append, value_delimiter = ',')]
    pub tags: Vec<String>,
    /// Include normally-hidden statuses for the queried kind. Task adds
    /// done/rejected/archived; ADR adds superseded; learning adds
    /// superseded; doc is a no-op.
    #[arg(long)]
    pub all: bool,
    /// Explicit per-kind status override (comma-separated set). Composes with
    /// --tag and --path; overrides --all.
    #[arg(long, value_delimiter = ',')]
    pub status: Vec<String>,
    /// Filter to artifacts applicable to this filesystem path. Task: selector
    /// containment over `context_files`. Learning: glob-containment over
    /// `scope.paths`. ADR: glob-containment over `paths`. Doc: out of scope
    /// (returns empty).
    #[arg(long)]
    pub path: Option<String>,
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SearchKindArg {
    Task,
    Doc,
    Learning,
    Adr,
    All,
}

impl std::fmt::Display for SearchKindArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Task => "task",
            Self::Doc => "doc",
            Self::Learning => "learning",
            Self::Adr => "adr",
            Self::All => "all",
        })
    }
}

impl From<SearchKindArg> for GlobalSearchKind {
    fn from(value: SearchKindArg) -> Self {
        match value {
            SearchKindArg::Task => Self::Task,
            SearchKindArg::Doc => Self::Doc,
            SearchKindArg::Learning => Self::Learning,
            SearchKindArg::Adr => Self::Adr,
            SearchKindArg::All => Self::All,
        }
    }
}

impl Execute for SearchCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let response = runtime.global_search(GlobalSearchParams {
            query: self.query,
            hybrid: self.hybrid,
            semantic: self.semantic,
            kind: self.kind.into(),
            limit: self.limit,
            field: self.field,
            model: self.model,
            tags: self.tags,
            all: self.all,
            status: self.status,
            path: self.path,
        })?;

        if self.json {
            crate::output::json::print_pretty(&serde_json::json!(response))
        } else {
            for note in &response.notes {
                eprintln!("note: {note}");
            }
            print_search_table(&response.results);
            Ok(())
        }
    }
}

fn print_search_table(results: &[GlobalSearchHit]) {
    let mut table =
        crate::output::table::build_table(&["KIND", "SOURCE", "ID/PATH", "TITLE/SUMMARY", "MATCH"]);
    for hit in results {
        table.add_row(vec![
            hit.kind.clone(),
            hit.source.clone(),
            hit.id.clone().or(hit.path.clone()).unwrap_or_default(),
            hit.title
                .clone()
                .or(hit.summary.clone())
                .unwrap_or_default(),
            match_text(hit),
        ]);
    }
    println!("{table}");
}

fn match_text(hit: &GlobalSearchHit) -> String {
    if let Some(field) = &hit.best_field {
        let score = hit.score.map(|score| format!(" score={score:.4}"));
        return format!("best={field}{}", score.unwrap_or_default());
    }
    hit.matched_by
        .as_ref()
        .map(|matched| matched.join(", "))
        .unwrap_or_default()
}
