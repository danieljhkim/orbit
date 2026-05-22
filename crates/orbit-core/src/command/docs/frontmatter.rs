use std::path::Path;

use orbit_common::types::OrbitError;

use super::path_util::component_str;
use super::types::{DocFrontmatter, DocType, FrontmatterBlock, ParsedDoc, RawDocFrontmatter};

pub fn parse_doc_frontmatter_strict(path: &Path, raw: &str) -> Result<DocFrontmatter, OrbitError> {
    parse_doc_strict(path, raw).map(|parsed| parsed.frontmatter)
}

pub(super) fn parse_doc_strict(path: &Path, raw: &str) -> Result<ParsedDoc, OrbitError> {
    let block = split_frontmatter(raw).map_err(|message| {
        OrbitError::InvalidInput(format!(
            "invalid frontmatter in {}: {message}",
            path.display()
        ))
    })?;
    let block = block.ok_or_else(|| {
        OrbitError::InvalidInput(format!("missing frontmatter block in {}", path.display()))
    })?;
    let raw_fm = serde_yaml::from_str::<RawDocFrontmatter>(block.raw).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "invalid frontmatter YAML in {}: {error}",
            path.display()
        ))
    })?;
    let doc_type = raw_fm.doc_type.ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "frontmatter in {} is missing required field `type`",
            path.display()
        ))
    })?;
    let summary = raw_fm.summary.ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "frontmatter in {} is missing required field `summary`",
            path.display()
        ))
    })?;
    let summary = summary.trim().to_string();
    if summary.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "frontmatter field `summary` in {} must not be empty",
            path.display()
        )));
    }
    if summary.lines().count() != 1 {
        return Err(OrbitError::InvalidInput(format!(
            "frontmatter field `summary` in {} must be a single line",
            path.display()
        )));
    }
    Ok(ParsedDoc {
        frontmatter: DocFrontmatter {
            doc_type,
            summary,
            tags: clean_string_list(raw_fm.tags),
            paths: clean_string_list(raw_fm.paths),
            related_features: clean_string_list(raw_fm.related_features),
            related_artifacts: raw_fm.related_artifacts,
        },
        body: block.body.to_string(),
    })
}

pub(super) fn parse_doc_tolerant(
    repo_relative: &Path,
    absolute_path: &Path,
    raw: &str,
) -> ParsedDoc {
    if let Ok(parsed) = parse_doc_strict(absolute_path, raw) {
        return parsed;
    }
    let body = split_frontmatter(raw)
        .ok()
        .flatten()
        .map(|block| block.body)
        .unwrap_or(raw);
    ParsedDoc {
        frontmatter: infer_frontmatter(repo_relative, body),
        body: body.to_string(),
    }
}

pub(super) fn split_frontmatter(raw: &str) -> Result<Option<FrontmatterBlock<'_>>, String> {
    let Some(first_line_end) = raw.find('\n') else {
        return Ok(None);
    };
    if raw[..first_line_end].trim_end_matches('\r') != "---" {
        return Ok(None);
    }
    let rest_start = first_line_end + 1;
    let mut cursor = rest_start;
    for line in raw[rest_start..].split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches('\n').trim_end_matches('\r');
        if line_without_newline == "---" {
            let body_start = cursor + line.len();
            return Ok(Some(FrontmatterBlock {
                raw: &raw[rest_start..cursor],
                body: &raw[body_start..],
            }));
        }
        cursor += line.len();
    }
    Err("unterminated frontmatter block".to_string())
}

fn clean_string_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

pub(super) fn infer_frontmatter(repo_relative: &Path, body: &str) -> DocFrontmatter {
    let (doc_type, tags) = infer_type_and_tags(repo_relative);
    DocFrontmatter {
        doc_type,
        summary: infer_summary(repo_relative, body),
        tags,
        paths: Vec::new(),
        related_features: Vec::new(),
        related_artifacts: Vec::new(),
    }
}

fn infer_type_and_tags(repo_relative: &Path) -> (DocType, Vec<String>) {
    let components = repo_relative
        .components()
        .filter_map(component_str)
        .collect::<Vec<_>>();
    if components.len() >= 4 && components[0] == "docs" && components[1] == "design" {
        return (DocType::Design, vec![components[2].to_string()]);
    }
    if components.len() >= 3 && components[0] == "docs" && components[1] == "design-patterns" {
        return (DocType::Pattern, Vec::new());
    }
    if components.contains(&"runbooks") {
        return (DocType::Runbook, Vec::new());
    }
    if repo_relative
        .file_stem()
        .and_then(|value| value.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("glossary"))
        || components.contains(&"glossary")
    {
        return (DocType::Glossary, Vec::new());
    }
    (DocType::Context, Vec::new())
}

fn infer_summary(repo_relative: &Path, body: &str) -> String {
    for line in body.lines() {
        let mut candidate = line.trim();
        if candidate.is_empty() || candidate == "---" {
            continue;
        }
        if candidate.starts_with("<!--") {
            continue;
        }
        candidate = candidate.trim_start_matches('#').trim();
        candidate = candidate.trim_matches('`').trim();
        if candidate.is_empty() {
            continue;
        }
        let candidate = candidate.trim_matches('<').trim_matches('>').trim();
        if !candidate.is_empty() {
            return candidate.to_string();
        }
    }
    repo_relative
        .file_stem()
        .and_then(|value| value.to_str())
        .map(titleize_slug)
        .unwrap_or_else(|| "Untitled document".to_string())
}

fn titleize_slug(raw: &str) -> String {
    raw.replace(['_', '-'], " ")
        .split_whitespace()
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
