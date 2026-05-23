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
