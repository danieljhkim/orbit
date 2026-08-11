//! Unit tests for docs lexical scoring.

use super::super::docs::*;

fn doc(path: &str, summary: &str) -> DocSearchSource {
    DocSearchSource {
        path: path.to_string(),
        doc_type: "design".to_string(),
        summary: summary.to_string(),
        tags: vec!["orbit-docs".to_string()],
        paths: Vec::new(),
        related_features: Vec::new(),
        related_artifacts: Vec::new(),
        body: "A complete decision body".to_string(),
    }
}

#[test]
fn score_doc_record_matches_inlined_body_and_returns_snippet() {
    let mut record = doc("docs/design/example/4_decisions.md", "Decision log");
    record.body = "Before the heliotrope-dispatch choice, requests were queued.".to_string();

    let result = score_doc_record(record, "heliotrope-dispatch").expect("body match");

    assert_eq!(result.matched_by, vec!["body"]);
    assert!(
        result
            .snippet
            .is_some_and(|snippet| snippet.contains("heliotrope-dispatch"))
    );
}

#[test]
fn score_doc_record_matches_summary_type_and_tags() {
    let summary =
        score_doc_record(doc("docs/a.md", "Inline ADR entries"), "adr").expect("summary match");
    assert_eq!(summary.matched_by, vec!["summary"]);

    let tag = score_doc_record(doc("docs/a.md", "Decisions"), "orbit-docs").expect("tag match");
    assert_eq!(tag.matched_by, vec!["tag:orbit-docs"]);

    let kind = score_doc_record(doc("docs/a.md", "Decisions"), "design").expect("type match");
    assert_eq!(kind.matched_by, vec!["type:design"]);
}

#[test]
fn sort_search_results_breaks_ties_by_path() {
    let mut results = vec![
        SearchResult::Doc(score_doc_record(doc("docs/b.md", "ADR body"), "adr").expect("b")),
        SearchResult::Doc(score_doc_record(doc("docs/a.md", "ADR body"), "adr").expect("a")),
    ];
    sort_search_results(&mut results);
    let paths = results
        .iter()
        .map(|result| match result {
            SearchResult::Doc(result) => result.record.path.as_str(),
        })
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["docs/a.md", "docs/b.md"]);
}
