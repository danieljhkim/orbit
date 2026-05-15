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
mod tests {
    use crate::graph::nodes::{
        BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode,
    };
    use crate::service::GraphContextService;

    use super::*;

    #[test]
    fn overview_auto_format_matches_fixture_snapshots() {
        let tiny = overview_body_snapshot(&fixture_graph(2), Some("src/"));
        let medium = overview_body_snapshot(&fixture_graph(21), Some("src/"));
        let large = overview_body_snapshot(&fixture_graph(60), None);

        assert_eq!(tiny, "full:1:2:2:false");
        assert_eq!(medium, "summary:1:21:21:false");
        assert_eq!(large, "summary:2:60:60:false");
    }

    #[test]
    fn requested_full_downgrades_large_scope() {
        let graph = fixture_graph(51);
        let svc = GraphContextService::new(&graph);
        let overview = svc.overview(None);
        let resolved = default_format_for_scope(None, overview.files.len());
        let downgraded = overview.files.len() > FILE_THRESHOLD;

        assert_eq!(resolved, OverviewFormat::Summary);
        assert!(downgraded);
    }

    #[test]
    fn requested_full_below_threshold_has_no_downgrade_reason() {
        let result = overview_result_for_fixture(FILE_THRESHOLD, None, Some(OverviewFormat::Full));

        assert!(matches!(result.body, OverviewBody::Full(_)));
    }

    #[test]
    fn requested_full_above_threshold_reports_file_threshold_reason() {
        let actual = 101;
        let result = overview_result_for_fixture(actual, None, Some(OverviewFormat::Full));

        let OverviewBody::Summary {
            summary,
            downgraded,
            downgrade_reason,
        } = result.body
        else {
            panic!("expected summary overview");
        };

        assert!(downgraded);
        assert_eq!(summary.total_files, actual);
        assert_eq!(
            downgrade_reason,
            Some(DowngradeReason::FileThreshold {
                threshold: FILE_THRESHOLD,
                actual,
            })
        );
        assert!(summary.hint.contains("file_threshold"));
        assert!(summary.hint.contains(&format!(
            "file count {actual} exceeds threshold {FILE_THRESHOLD}"
        )));
    }

    #[test]
    fn requested_summary_above_threshold_has_no_downgrade_reason() {
        let result = overview_result_for_fixture(101, None, Some(OverviewFormat::Summary));

        let OverviewBody::Summary {
            downgraded,
            downgrade_reason,
            ..
        } = result.body
        else {
            panic!("expected summary overview");
        };

        assert!(!downgraded);
        assert_eq!(downgrade_reason, None);
    }

    fn overview_body_snapshot(graph: &CodebaseGraphV1, prefix: Option<&str>) -> String {
        let svc = GraphContextService::new(graph);
        let overview = svc.overview(prefix);
        let resolved = default_format_for_scope(prefix, overview.files.len());
        let downgraded = false;
        if matches!(resolved, OverviewFormat::Summary) {
            let summary = compact_from_overview(&overview, prefix, SUMMARY_HINT);
            format!(
                "summary:{}:{}:{}:{}",
                summary.total_dirs, summary.total_files, summary.total_symbols, downgraded
            )
        } else {
            format!(
                "full:{}:{}:{}:{}",
                overview.total_dirs, overview.total_files, overview.total_symbols, downgraded
            )
        }
    }

    fn overview_result_for_fixture(
        file_count: usize,
        prefix: Option<&str>,
        input_format: Option<OverviewFormat>,
    ) -> OverviewResult {
        let graph = fixture_graph(file_count);
        let svc = GraphContextService::new(&graph);
        let overview = svc.overview(prefix);
        result_from_overview(
            overview,
            prefix,
            input_format,
            requested_format(input_format),
        )
    }

    fn fixture_graph(file_count: usize) -> CodebaseGraphV1 {
        let root_id = "dir:.".to_string();
        let src_id = "dir:src".to_string();
        let mut file_ids = Vec::with_capacity(file_count);
        let mut files = Vec::with_capacity(file_count);
        let mut leaves = Vec::with_capacity(file_count);

        for index in 0..file_count {
            let file_id = format!("file:src/file_{index:03}.rs");
            let file_path = format!("src/file_{index:03}.rs");
            let leaf_id = format!("symbol:{file_path}#symbol_{index}:function");
            file_ids.push(file_id.clone());
            files.push(FileNode {
                base: base_node(
                    &file_id,
                    &format!("file_{index:03}.rs"),
                    &file_path,
                    "rust",
                    Some(&src_id),
                ),
                extension: Some("rs".to_string()),
                source_blob_hash: None,
                source: String::new(),
                imports: Vec::new(),
                exports: Vec::new(),
                re_exports: Vec::new(),
                leaf_children: vec![leaf_id.clone()],
            });
            leaves.push(LeafNode {
                base: base_node(
                    &leaf_id,
                    &format!("symbol_{index}"),
                    &format!("{file_path}#symbol_{index}"),
                    "rust",
                    Some(&file_id),
                ),
                kind: LeafKind::Function,
                source: String::new(),
                source_blob_hash: None,
                source_hash: None,
                file_hash_at_capture: None,
                history: Vec::new(),
                input_signature: Vec::new(),
                output_signature: Vec::new(),
                start_line: Some(1),
                end_line: Some(1),
                children: Vec::new(),
            });
        }

        CodebaseGraphV1 {
            root_dir_id: root_id.clone(),
            dirs: vec![
                DirNode {
                    base: base_node(&root_id, ".", ".", "", None),
                    dir_children: vec![src_id.clone()],
                    file_children: Vec::new(),
                },
                DirNode {
                    base: base_node(&src_id, "src", "src/", "", Some(&root_id)),
                    dir_children: Vec::new(),
                    file_children: file_ids,
                },
            ],
            files,
            leaves,
        }
    }

    fn base_node(
        id: &str,
        name: &str,
        location: &str,
        language: &str,
        parent_id: Option<&str>,
    ) -> BaseNodeFields {
        BaseNodeFields {
            id: id.to_string(),
            identity_key: id.to_string(),
            object_hash: None,
            name: name.to_string(),
            location: location.to_string(),
            language: language.to_string(),
            description: String::new(),
            parent_id: parent_id.map(str::to_string),
            is_locked: false,
            lineage_locked: false,
            lock_owner: None,
            lock_reason: String::new(),
        }
    }
}
