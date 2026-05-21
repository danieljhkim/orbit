//! Unit tests for `docs` (lexical) — sibling layout under lexical/tests/.

use std::path::PathBuf;

use orbit_common::types::AdrStatus;

use super::super::docs::*;

fn adr_fixture(
    id: &str,
    title: &str,
    status: AdrStatus,
    related_features: Vec<&str>,
) -> AdrSearchSource {
    AdrSearchSource {
        id: id.to_string(),
        title: title.to_string(),
        status,
        path: PathBuf::from(".orbit")
            .join("adrs")
            .join(status.cli_name())
            .join(id)
            .join("body.md"),
        tags: Vec::new(),
        paths: Vec::new(),
        related_features: related_features
            .into_iter()
            .map(ToString::to_string)
            .collect(),
    }
}

#[test]
fn score_adr_record_exercises_title_feature_and_status_branches() {
    let title = score_adr_record(
        adr_fixture(
            "ADR-0001",
            "Docs federation overlay",
            AdrStatus::Accepted,
            vec![],
        ),
        "federation",
    )
    .expect("title match");
    assert_eq!(title.score, 90);
    assert_eq!(title.matched_by, vec!["title"]);

    let exact_feature = score_adr_record(
        adr_fixture(
            "ADR-0002",
            "Boundary",
            AdrStatus::Accepted,
            vec!["orbit-docs"],
        ),
        "orbit-docs",
    )
    .expect("exact feature match");
    assert_eq!(exact_feature.score, 120);
    assert_eq!(exact_feature.matched_by, vec!["related_feature:orbit-docs"]);

    let substring_feature = score_adr_record(
        adr_fixture(
            "ADR-0003",
            "Boundary",
            AdrStatus::Accepted,
            vec!["orbit-docs"],
        ),
        "docs",
    )
    .expect("substring feature match");
    assert_eq!(substring_feature.score, 60);
    assert_eq!(
        substring_feature.matched_by,
        vec!["related_feature:orbit-docs"]
    );

    let status = score_adr_record(
        adr_fixture("ADR-0004", "Boundary", AdrStatus::Proposed, vec![]),
        "proposed",
    )
    .expect("status match");
    assert_eq!(status.score, 30);
    assert_eq!(status.matched_by, vec!["status:proposed"]);

    assert!(
        score_adr_record(
            adr_fixture("ADR-0005", "Boundary", AdrStatus::Accepted, vec![]),
            "missing",
        )
        .is_none()
    );
}

#[test]
fn sort_search_results_breaks_adr_ties_by_ascending_id() {
    let mut results = vec![
        SearchResult::Adr(
            score_adr_record(
                adr_fixture(
                    "ADR-0002",
                    "Boundary",
                    AdrStatus::Accepted,
                    vec!["orbit-docs"],
                ),
                "orbit-docs",
            )
            .expect("second"),
        ),
        SearchResult::Adr(
            score_adr_record(
                adr_fixture(
                    "ADR-0001",
                    "Boundary",
                    AdrStatus::Accepted,
                    vec!["orbit-docs"],
                ),
                "orbit-docs",
            )
            .expect("first"),
        ),
    ];

    sort_search_results(&mut results);

    let ids = results
        .iter()
        .map(|result| match result {
            SearchResult::Adr(result) => result.id.as_str(),
            SearchResult::Doc(_) => panic!("expected only ADR results"),
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["ADR-0001", "ADR-0002"]);
}

#[test]
fn score_adr_record_matches_tags() {
    let mut adr = adr_fixture(
        "ADR-0001",
        "Boundary",
        AdrStatus::Accepted,
        vec!["orbit-docs"],
    );
    adr.tags = vec!["adr-schema".to_string(), "Cross-Cutting".to_string()];

    let result = score_adr_record(adr, "cross-cutting").expect("tag match");

    assert_eq!(result.score, 120);
    assert_eq!(result.matched_by, vec!["tag:Cross-Cutting"]);
}

#[test]
fn adr_paths_containment_matches_positive_and_negative_cases() {
    let paths = vec![
        "crates/orbit-search/**".to_string(),
        "docs/design/adr-artifact/**".to_string(),
    ];

    assert!(
        adr_paths_contain_path(&paths, "crates/orbit-search/src/lib.rs").expect("positive match")
    );
    assert!(
        !adr_paths_contain_path(&paths, "crates/orbit-core/src/lib.rs").expect("negative match")
    );
}
