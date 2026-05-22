use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use orbit_common::types::{Adr, AdrStatus, OrbitError, Task};
use orbit_common::utility::glob::{match_glob, normalize_glob_path};
use orbit_common::utility::selector::anchor_path;
use orbit_search::{
    score_adr_record, score_doc_record, sort_search_results, AdrEmbeddingSource, AdrSearchSource,
    DocEmbeddingSource, DocSearchSource,
};

use super::frontmatter::parse_doc_tolerant;
use super::path_util::{path_to_slash_string, repo_relative_path};
use super::types::{DocRecord, DocShow, DocType, TaskRelatedDoc};
use super::walk::{expand_root, walk_docs_roots};

const DEFAULT_RELATED_DOC_LIMIT: usize = 5;

pub(super) fn show_doc(
    repo_root: &Path,
    roots: &[String],
    requested: &str,
) -> Result<DocShow, OrbitError> {
    let requested_path = Path::new(requested.trim());
    let absolute = if requested_path.is_absolute() {
        requested_path.to_path_buf()
    } else {
        repo_root.join(requested_path)
    };
    if !absolute.is_file() {
        return Err(OrbitError::InvalidInput(format!(
            "docs path does not exist or is not a file: {requested}"
        )));
    }
    if path_is_or_contains_dot_orbit(repo_root, &absolute) {
        return Err(OrbitError::InvalidInput(
            "docs paths under .orbit/ are not indexed by orbit-docs".to_string(),
        ));
    }
    if !path_is_under_configured_roots(repo_root, roots, &absolute)? {
        return Err(OrbitError::InvalidInput(format!(
            "docs path is outside configured [docs].roots: {requested}"
        )));
    }
    let relative = repo_relative_path(repo_root, &absolute)?;
    let raw = std::fs::read_to_string(&absolute)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", absolute.display())))?;
    let parsed = parse_doc_tolerant(&relative, &absolute, &raw);
    Ok(DocShow {
        path: path_to_slash_string(&relative),
        frontmatter: parsed.frontmatter,
        body: parsed.body,
    })
}

fn path_is_or_contains_dot_orbit(repo_root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(repo_root).unwrap_or(path);
    relative.components().any(
        |component| matches!(component, std::path::Component::Normal(value) if value == ".orbit"),
    )
}

pub(super) fn path_is_under_configured_roots(
    repo_root: &Path,
    roots: &[String],
    path: &Path,
) -> Result<bool, OrbitError> {
    let canonical_path = path
        .canonicalize()
        .map_err(|error| OrbitError::Io(format!("canonicalize {}: {error}", path.display())))?;
    for root in roots {
        for root_path in expand_root(repo_root, root)? {
            if path_is_or_contains_dot_orbit(repo_root, &root_path) {
                continue;
            }
            if root_path.is_file() {
                let canonical_root = root_path.canonicalize().map_err(|error| {
                    OrbitError::Io(format!("canonicalize {}: {error}", root_path.display()))
                })?;
                if canonical_path == canonical_root {
                    return Ok(true);
                }
            } else if root_path.is_dir() {
                let canonical_root = root_path.canonicalize().map_err(|error| {
                    OrbitError::Io(format!("canonicalize {}: {error}", root_path.display()))
                })?;
                if canonical_path.starts_with(canonical_root) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

pub(super) fn adr_status_in_docs_search(status: AdrStatus, include_superseded: bool) -> bool {
    matches!(status, AdrStatus::Proposed | AdrStatus::Accepted)
        || (include_superseded && status == AdrStatus::Superseded)
}

pub(super) fn doc_search_source(record: DocRecord) -> DocSearchSource {
    DocSearchSource {
        path: record.path,
        doc_type: record.frontmatter.doc_type.as_str().to_string(),
        summary: record.frontmatter.summary,
        tags: record.frontmatter.tags,
        paths: record.frontmatter.paths,
        related_features: record.frontmatter.related_features,
        related_artifacts: record
            .frontmatter
            .related_artifacts
            .into_iter()
            .map(|artifact| artifact.as_str().to_string())
            .collect(),
    }
}

pub(super) fn doc_embedding_sources(
    repo_root: &Path,
    roots: &[String],
) -> Result<Vec<DocEmbeddingSource>, OrbitError> {
    let mut sources = Vec::new();
    for record in walk_docs_roots(repo_root, roots)? {
        let shown = show_doc(repo_root, roots, &record.path)?;
        sources.push(DocEmbeddingSource {
            path: record.path,
            title: shown.frontmatter.summary,
            tags: shown.frontmatter.tags,
            body: shown.body,
        });
    }
    Ok(sources)
}

pub(super) fn adr_embedding_sources(
    repo_root: &Path,
    adrs: Vec<Adr>,
) -> Result<Vec<AdrEmbeddingSource>, OrbitError> {
    let mut sources = Vec::new();
    // L-0028: ADR ids can briefly appear in multiple status directories; index one source per id.
    let adrs = adrs
        .into_iter()
        .map(|adr| (adr.id.clone(), adr))
        .collect::<BTreeMap<_, _>>();
    for adr in adrs.into_values() {
        let body_path = repo_root.join(adr_body_search_path(adr.status, &adr.id));
        let body = std::fs::read_to_string(&body_path)
            .map_err(|error| OrbitError::Io(format!("read {}: {error}", body_path.display())))?;
        sources.push(AdrEmbeddingSource {
            id: adr.id,
            title: adr.title,
            body,
            tags: adr.tags,
        });
    }
    Ok(sources)
}

pub(super) fn adr_search_source(adr: Adr) -> AdrSearchSource {
    AdrSearchSource {
        path: adr_body_search_path(adr.status, &adr.id),
        id: adr.id,
        title: adr.title,
        status: adr.status,
        tags: adr.tags,
        paths: adr.paths,
        related_features: adr.related_features,
    }
}

pub(super) fn adr_body_search_path(status: AdrStatus, id: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(".orbit")
        .join("adrs")
        .join(status.cli_name())
        .join(id)
        .join("body.md")
}

#[derive(Debug)]
struct RelatedDocCandidate {
    record: DocRecord,
    score: usize,
    matched_by: BTreeSet<String>,
}

pub(super) fn related_docs_for_context(
    repo_root: &Path,
    roots: &[String],
    context_files: &[String],
    related_features: &[String],
    limit: Option<usize>,
) -> Result<Vec<TaskRelatedDoc>, OrbitError> {
    let limit = limit.unwrap_or(DEFAULT_RELATED_DOC_LIMIT);
    if limit == 0 {
        return Ok(Vec::new());
    }

    let context_paths = context_files
        .iter()
        .filter_map(|selector| context_selector_path(repo_root, selector))
        .collect::<Vec<_>>();
    let features = related_features
        .iter()
        .map(|feature| feature.trim().to_ascii_lowercase())
        .filter(|feature| !feature.is_empty())
        .collect::<BTreeSet<_>>();
    if context_paths.is_empty() && features.is_empty() {
        return Ok(Vec::new());
    }

    let mut candidates = BTreeMap::<String, RelatedDocCandidate>::new();
    for record in walk_docs_roots(repo_root, roots)? {
        let mut score = 0usize;
        let mut matched_by = BTreeSet::new();

        for glob in &record.frontmatter.paths {
            let Some(normalized_glob) = normalize_doc_path_glob(glob) else {
                continue;
            };
            for context_path in &context_paths {
                if doc_path_glob_matches_context(&normalized_glob, context_path)? {
                    score += 200 + normalized_glob.len();
                    matched_by.insert(format!("path:{glob}"));
                    break;
                }
            }
        }

        for feature in &record.frontmatter.related_features {
            let normalized = feature.trim().to_ascii_lowercase();
            if !normalized.is_empty() && features.contains(&normalized) {
                score += 160 + normalized.len();
                matched_by.insert(format!("feature:{feature}"));
            }
        }

        if score == 0 {
            continue;
        }

        candidates
            .entry(record.path.clone())
            .and_modify(|candidate| {
                candidate.score += score;
                candidate.matched_by.extend(matched_by.iter().cloned());
            })
            .or_insert(RelatedDocCandidate {
                record,
                score,
                matched_by,
            });
    }

    let mut ranked = candidates.into_values().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.record.path.cmp(&right.record.path))
    });
    ranked.truncate(limit);

    ranked
        .into_iter()
        .map(|candidate| {
            let shown = show_doc(repo_root, roots, &candidate.record.path)?;
            Ok(TaskRelatedDoc {
                path: candidate.record.path,
                doc_type: candidate.record.frontmatter.doc_type,
                summary: candidate.record.frontmatter.summary.clone(),
                excerpt: doc_excerpt(&shown.body, &candidate.record.frontmatter.summary),
                matched_by: candidate.matched_by.into_iter().collect(),
            })
        })
        .collect()
}

fn context_selector_path(repo_root: &Path, selector: &str) -> Option<String> {
    let anchor = anchor_path(selector).ok()?;
    let relative = if anchor.is_absolute() {
        anchor.strip_prefix(repo_root).ok()?.to_path_buf()
    } else {
        anchor
    };
    normalize_glob_path(&path_to_slash_string(&relative)).ok()
}

fn normalize_doc_path_glob(glob: &str) -> Option<String> {
    normalize_glob_path(glob).ok()
}

fn doc_path_glob_matches_context(glob: &str, context_path: &str) -> Result<bool, OrbitError> {
    if match_glob(glob, context_path)? {
        return Ok(true);
    }
    let literal_prefix = glob.trim_end_matches('/');
    Ok(!contains_glob_operator(literal_prefix)
        && context_path
            .strip_prefix(literal_prefix)
            .is_some_and(|rest| rest.starts_with('/')))
}

fn contains_glob_operator(value: &str) -> bool {
    value.contains('*') || value.contains('?')
}

fn doc_excerpt(body: &str, fallback: &str) -> String {
    for line in body.lines() {
        let trimmed = line
            .trim()
            .trim_start_matches('#')
            .trim()
            .trim_matches('`')
            .trim();
        if !trimmed.is_empty() && trimmed != "---" {
            return truncate_excerpt(trimmed);
        }
    }
    truncate_excerpt(fallback)
}

fn truncate_excerpt(value: &str) -> String {
    const MAX_EXCERPT_CHARS: usize = 160;
    let mut out = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index == MAX_EXCERPT_CHARS {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}
