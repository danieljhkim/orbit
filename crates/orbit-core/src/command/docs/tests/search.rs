//! Search result shape and related_docs matching tests migrated for ORB-00250.

use std::fs;

use tempfile::tempdir;

use super::super::search::related_docs_for_context;
use super::super::types::DocType;

use orbit_search::{DocSearchResult, DocSearchSource, SearchResult};
use serde_json::json;

#[test]
fn search_result_doc_json_shape_matches_legacy_flat_record() {
    let result = SearchResult::Doc(DocSearchResult {
        record: DocSearchSource {
            path: "docs/pattern.md".to_string(),
            doc_type: "pattern".to_string(),
            summary: "RAII guard pattern".to_string(),
            tags: vec!["rust".to_string(), "guard".to_string()],
            paths: Vec::new(),
            related_features: Vec::new(),
            related_artifacts: vec!["ORB-00160".to_string()],
        },
        score: 84,
        matched_by: vec!["summary".to_string()],
    });

    let actual = serde_json::to_value(&result).expect("serialize search result");

    assert_eq!(
        actual,
        json!({
            "Doc": {
                "path": "docs/pattern.md",
                "type": "pattern",
                "summary": "RAII guard pattern",
                "tags": ["rust", "guard"],
                "related_artifacts": ["ORB-00160"],
                "score": 84,
                "matched_by": ["summary"]
            }
        })
    );
}

#[test]
fn related_docs_match_context_files_against_doc_paths() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("docs")).expect("docs dir");
    fs::write(
        root.join("docs/cli.md"),
        "---\ntype: design\nsummary: CLI command design\npaths: [\"crates/orbit-cli/**\"]\n---\n# CLI Commands\n\nBody\n",
    )
    .expect("write doc");

    let related = related_docs_for_context(
        root,
        &["docs/".to_string()],
        &["file:crates/orbit-cli/src/command/docs.rs".to_string()],
        &[],
        Some(5),
    )
    .expect("related docs");

    assert_eq!(related.len(), 1);
    assert_eq!(related[0].path, "docs/cli.md");
    assert_eq!(related[0].doc_type, DocType::Design);
    assert_eq!(related[0].excerpt, "CLI Commands");
    assert_eq!(related[0].matched_by, vec!["path:crates/orbit-cli/**"]);
}

#[test]
fn related_docs_match_task_features_against_doc_related_features() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("docs")).expect("docs dir");
    fs::write(
        root.join("docs/orbit-docs.md"),
        "---\ntype: context\nsummary: Orbit docs context\nrelated_features: [orbit-docs]\n---\nTask-time docs injection\n",
    )
    .expect("write doc");

    let related = related_docs_for_context(
        root,
        &["docs/".to_string()],
        &[],
        &["Orbit-Docs".to_string()],
        Some(5),
    )
    .expect("related docs");

    assert_eq!(related.len(), 1);
    assert_eq!(related[0].path, "docs/orbit-docs.md");
    assert_eq!(related[0].matched_by, vec!["feature:orbit-docs"]);
}
