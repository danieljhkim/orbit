#![allow(missing_docs)]

use crate::graph::nodes::{BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode};
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
