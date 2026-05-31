use std::str::FromStr;

use serde::Serialize;

use super::DEFAULT_LIMIT;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GlobalSearchKind {
    Task,
    Doc,
    Learning,
    Adr,
    #[default]
    All,
}

impl GlobalSearchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Doc => "doc",
            Self::Learning => "learning",
            Self::Adr => "adr",
            Self::All => "all",
        }
    }

    pub(super) fn includes_tasks(self) -> bool {
        matches!(self, Self::Task | Self::All)
    }

    pub(super) fn includes_docs(self) -> bool {
        matches!(self, Self::Doc | Self::All)
    }

    pub(super) fn includes_learnings(self) -> bool {
        matches!(self, Self::Learning | Self::All)
    }

    pub(super) fn includes_adrs(self) -> bool {
        matches!(self, Self::Adr | Self::All)
    }
}

impl FromStr for GlobalSearchKind {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "task" => Ok(Self::Task),
            "doc" => Ok(Self::Doc),
            "learning" => Ok(Self::Learning),
            "adr" => Ok(Self::Adr),
            "all" => Ok(Self::All),
            other => Err(format!(
                "invalid search kind `{other}`; expected one of: task, doc, learning, adr, all"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GlobalSearchMode {
    Lexical,
    Hybrid,
    Neighbor,
}

#[derive(Debug, Clone, Default)]
pub struct GlobalSearchParams {
    pub query: Option<String>,
    // ADR-0179: hybrid free-text ranking and task-neighbor lookup are distinct modes.
    pub hybrid: bool,
    pub semantic: Option<String>,
    pub kind: GlobalSearchKind,
    pub limit: usize,
    /// AND-filter by tag. Repeat for multi-tag AND semantics. Applies to
    /// task, doc, learning, ADR (and `all`).
    pub tags: Vec<String>,
    /// Include normally-hidden statuses for the queried kind(s). Mutually
    /// overridden by `status`.
    pub all: bool,
    /// Explicit per-kind status override (set semantics). When non-empty,
    /// takes precedence over the `all` widener.
    pub status: Vec<String>,
    /// Cross-kind applicability filter. Task: selector-mapping against
    /// `context_files`. Learning and ADR: glob-containment against
    /// applicability path globs. Doc: out of scope (returns empty).
    pub path: Option<String>,
}

impl GlobalSearchParams {
    pub fn normalized_limit(&self) -> usize {
        if self.limit == 0 {
            DEFAULT_LIMIT
        } else {
            self.limit
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GlobalSearchResponse {
    pub mode: GlobalSearchMode,
    pub kind: GlobalSearchKind,
    pub results: Vec<GlobalSearchHit>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GlobalSearchHit {
    pub kind: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_by: Option<Vec<String>>,
}
