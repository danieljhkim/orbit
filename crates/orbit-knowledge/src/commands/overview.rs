use crate::KnowledgeError;
use crate::commands::GraphCommandContext;
use crate::graph::GraphReadOptions;
use crate::service::{GraphContextService, TopFileEntry, compact_from_overview};

pub use crate::service::{GraphOverview, GraphOverviewSummary};

const AUTO_SUMMARY_FILE_THRESHOLD: usize = 20;
const FILE_THRESHOLD: usize = 50;
pub const SUMMARY_HINT: &str =
    "Use `prefix` to narrow the overview and get per-file symbol listings.";

/// Machine-readable reason an overview request was downgraded from full to summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DowngradeReason {
    /// The requested full overview exceeded the maximum file count for full mode.
    FileThreshold {
        /// Maximum file count allowed before full mode is downgraded.
        threshold: usize,
        /// Actual file count observed for the requested scope.
        actual: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverviewFormat {
    Full,
    Summary,
}

#[derive(Debug, Clone)]
pub struct OverviewInput {
    pub context: GraphCommandContext,
    pub prefix: Option<String>,
    pub requested_format: Option<OverviewFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestedOverviewFormat {
    Auto,
    Full,
    Summary,
}

impl RequestedOverviewFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Full => "full",
            Self::Summary => "summary",
        }
    }
}

pub struct OverviewResult {
    pub requested_format: RequestedOverviewFormat,
    pub body: OverviewBody,
}

pub enum OverviewBody {
    Full(GraphOverview),
    Summary {
        summary: GraphOverviewSummary,
        downgraded: bool,
        downgrade_reason: Option<DowngradeReason>,
    },
}

pub fn run(input: OverviewInput) -> Result<OverviewResult, KnowledgeError> {
    let requested_format = requested_format(input.requested_format);
    if let Some(summary) = try_summary_via_sql_index(
        &input.context,
        input.prefix.as_deref(),
        input.requested_format,
    )? {
        return Ok(OverviewResult {
            requested_format,
            body: OverviewBody::Summary {
                summary: summary.summary,
                downgraded: summary.downgraded,
                downgrade_reason: summary.downgrade_reason,
            },
        });
    }

    let graph = input.context.read_graph(GraphReadOptions::default())?;
    let svc = GraphContextService::new(&graph);
    let overview = svc.overview(input.prefix.as_deref());
    Ok(result_from_overview(
        overview,
        input.prefix.as_deref(),
        input.requested_format,
        requested_format,
    ))
}

fn result_from_overview(
    overview: GraphOverview,
    prefix: Option<&str>,
    input_format: Option<OverviewFormat>,
    requested_format: RequestedOverviewFormat,
) -> OverviewResult {
    let resolved_format =
        input_format.unwrap_or_else(|| default_format_for_scope(prefix, overview.files.len()));
    let downgrade_reason = downgrade_reason(input_format, overview.files.len());
    let downgraded = downgrade_reason.is_some();
    let use_summary = matches!(resolved_format, OverviewFormat::Summary) || downgraded;

    if use_summary {
        let hint = summary_hint(downgrade_reason.as_ref());
        OverviewResult {
            requested_format,
            body: OverviewBody::Summary {
                summary: compact_from_overview(&overview, prefix, &hint),
                downgraded,
                downgrade_reason,
            },
        }
    } else {
        OverviewResult {
            requested_format,
            body: OverviewBody::Full(overview),
        }
    }
}

fn requested_format(format: Option<OverviewFormat>) -> RequestedOverviewFormat {
    match format {
        Some(OverviewFormat::Full) => RequestedOverviewFormat::Full,
        Some(OverviewFormat::Summary) => RequestedOverviewFormat::Summary,
        None => RequestedOverviewFormat::Auto,
    }
}

fn default_format_for_scope(prefix: Option<&str>, file_count: usize) -> OverviewFormat {
    if prefix.is_none() || file_count > AUTO_SUMMARY_FILE_THRESHOLD {
        OverviewFormat::Summary
    } else {
        OverviewFormat::Full
    }
}

fn downgrade_reason(
    requested_format: Option<OverviewFormat>,
    file_count: usize,
) -> Option<DowngradeReason> {
    if matches!(requested_format, Some(OverviewFormat::Full)) && file_count > FILE_THRESHOLD {
        Some(DowngradeReason::FileThreshold {
            threshold: FILE_THRESHOLD,
            actual: file_count,
        })
    } else {
        None
    }
}

fn summary_hint(downgrade_reason: Option<&DowngradeReason>) -> String {
    match downgrade_reason {
        Some(DowngradeReason::FileThreshold { threshold, actual }) => format!(
            "Downgrade reason file_threshold: file count {actual} exceeds threshold {threshold}. {SUMMARY_HINT}"
        ),
        None => SUMMARY_HINT.to_string(),
    }
}

struct SqlOverviewSummary {
    summary: GraphOverviewSummary,
    downgraded: bool,
    downgrade_reason: Option<DowngradeReason>,
}

fn try_summary_via_sql_index(
    context: &GraphCommandContext,
    prefix: Option<&str>,
    requested_format: Option<OverviewFormat>,
) -> Result<Option<SqlOverviewSummary>, KnowledgeError> {
    if prefix.is_some() {
        return Ok(None);
    }

    let Some(reader) = context.open_current_graph_index()? else {
        return Ok(None);
    };
    let (total_dirs, total_files, total_symbols) = reader.overview_counts().map_err(|error| {
        KnowledgeError::knowledge_unavailable(format!("query graph sqlite overview: {error}"))
    })?;
    let resolved_format =
        requested_format.unwrap_or_else(|| default_format_for_scope(None, total_files));
    let downgrade_reason = downgrade_reason(requested_format, total_files);
    let downgraded = downgrade_reason.is_some();
    let use_summary = matches!(resolved_format, OverviewFormat::Summary) || downgraded;
    if !use_summary {
        return Ok(None);
    }

    let top_files = reader
        .overview_top_files(10)
        .map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!("query graph sqlite overview: {error}"))
        })?
        .into_iter()
        .map(|(selector, name, symbol_count)| TopFileEntry {
            selector,
            name,
            symbol_count,
        })
        .collect();
    let hint = summary_hint(downgrade_reason.as_ref());

    Ok(Some(SqlOverviewSummary {
        summary: GraphOverviewSummary {
            total_dirs,
            total_files,
            total_symbols,
            languages: reader.overview_language_counts().map_err(|error| {
                KnowledgeError::knowledge_unavailable(format!(
                    "query graph sqlite overview: {error}"
                ))
            })?,
            symbol_kinds: reader.overview_symbol_kind_counts().map_err(|error| {
                KnowledgeError::knowledge_unavailable(format!(
                    "query graph sqlite overview: {error}"
                ))
            })?,
            dir_file_counts: reader.overview_dir_file_counts().map_err(|error| {
                KnowledgeError::knowledge_unavailable(format!(
                    "query graph sqlite overview: {error}"
                ))
            })?,
            top_files,
            hint,
        },
        downgraded,
        downgrade_reason,
    }))
}

#[cfg(test)]
mod tests;
