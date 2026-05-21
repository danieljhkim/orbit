use std::path::PathBuf;

use orbit_common::types::{AdrStatus, OrbitError};
use orbit_common::utility::glob::{compile_glob_regex, normalize_glob_path};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocSearchSource {
    pub path: String,
    #[serde(rename = "type")]
    pub doc_type: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_features: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_artifacts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AdrSearchSource {
    pub id: String,
    pub title: String,
    pub status: AdrStatus,
    pub path: PathBuf,
    pub tags: Vec<String>,
    pub paths: Vec<String>,
    pub related_features: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocSearchResult {
    #[serde(flatten)]
    pub record: DocSearchSource,
    pub score: usize,
    pub matched_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum SearchResult {
    Doc(DocSearchResult),
    Adr(AdrSearchResult),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AdrSearchResult {
    pub id: String,
    pub title: String,
    pub status: AdrStatus,
    pub path: PathBuf,
    pub tags: Vec<String>,
    pub paths: Vec<String>,
    pub related_features: Vec<String>,
    pub score: usize,
    pub matched_by: Vec<String>,
}

pub fn score_doc_record(record: DocSearchSource, query_lower: &str) -> Option<DocSearchResult> {
    let mut score = 0usize;
    let mut matched_by = Vec::new();
    let summary = record.summary.to_ascii_lowercase();
    if summary.contains(query_lower) {
        score += 80 + query_lower.len();
        matched_by.push("summary".to_string());
    }
    if record.doc_type.contains(query_lower) {
        score += 30;
        matched_by.push(format!("type:{}", record.doc_type));
    }
    for tag in &record.tags {
        let lower = tag.to_ascii_lowercase();
        if lower == query_lower {
            score += 120;
            matched_by.push(format!("tag:{tag}"));
        } else if lower.contains(query_lower) {
            score += 60;
            matched_by.push(format!("tag:{tag}"));
        }
    }
    if score == 0 {
        return None;
    }
    Some(DocSearchResult {
        record,
        score,
        matched_by,
    })
}

pub fn score_adr_record(adr: AdrSearchSource, query_lower: &str) -> Option<AdrSearchResult> {
    let mut score = 0usize;
    let mut matched_by = Vec::new();
    let title = adr.title.to_ascii_lowercase();
    if title.contains(query_lower) {
        score += 80 + query_lower.len();
        matched_by.push("title".to_string());
    }
    for feature in &adr.related_features {
        let lower = feature.to_ascii_lowercase();
        if lower == query_lower {
            score += 120;
            matched_by.push(format!("related_feature:{feature}"));
        } else if lower.contains(query_lower) {
            score += 60;
            matched_by.push(format!("related_feature:{feature}"));
        }
    }
    for tag in &adr.tags {
        let lower = tag.to_ascii_lowercase();
        if lower == query_lower {
            score += 120;
            matched_by.push(format!("tag:{tag}"));
        } else if lower.contains(query_lower) {
            score += 60;
            matched_by.push(format!("tag:{tag}"));
        }
    }
    let status = adr.status.cli_name();
    if status.contains(query_lower) {
        score += 30;
        matched_by.push(format!("status:{status}"));
    }
    if score == 0 {
        return None;
    }
    Some(AdrSearchResult {
        id: adr.id,
        title: adr.title,
        status: adr.status,
        path: adr.path,
        tags: adr.tags,
        paths: adr.paths,
        related_features: adr.related_features,
        score,
        matched_by,
    })
}

pub fn adr_paths_contain_path(rules: &[String], query_path: &str) -> Result<bool, OrbitError> {
    let normalized = normalize_glob_path(query_path)?;
    Ok(rules.iter().any(|rule| {
        compile_glob_regex(rule)
            .map(|regex| regex.is_match(&normalized))
            .unwrap_or(false)
    }))
}

pub fn sort_search_results(results: &mut [SearchResult]) {
    results.sort_by(|left, right| {
        search_result_score(right)
            .cmp(&search_result_score(left))
            .then_with(|| match (left, right) {
                (SearchResult::Doc(left), SearchResult::Doc(right)) => {
                    left.record.path.cmp(&right.record.path)
                }
                (SearchResult::Adr(left), SearchResult::Adr(right)) => left.id.cmp(&right.id),
                (SearchResult::Doc(_), SearchResult::Adr(_)) => std::cmp::Ordering::Less,
                (SearchResult::Adr(_), SearchResult::Doc(_)) => std::cmp::Ordering::Greater,
            })
    });
}

fn search_result_score(result: &SearchResult) -> usize {
    match result {
        SearchResult::Doc(result) => result.score,
        SearchResult::Adr(result) => result.score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            adr_paths_contain_path(&paths, "crates/orbit-search/src/lib.rs")
                .expect("positive match")
        );
        assert!(
            !adr_paths_contain_path(&paths, "crates/orbit-core/src/lib.rs")
                .expect("negative match")
        );
    }
}
