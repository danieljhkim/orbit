use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use orbit_common::types::OrbitError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::json;

use crate::OrbitRuntime;

const DEFAULT_DOC_ROOT: &str = "docs/";
const DOC_TYPES: &[&str] = &["design", "pattern", "context", "glossary", "runbook"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DocType {
    Design,
    Pattern,
    Context,
    Glossary,
    Runbook,
}

impl DocType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Design => "design",
            Self::Pattern => "pattern",
            Self::Context => "context",
            Self::Glossary => "glossary",
            Self::Runbook => "runbook",
        }
    }
}

impl fmt::Display for DocType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DocType {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim() {
            "design" => Ok(Self::Design),
            "pattern" => Ok(Self::Pattern),
            "context" => Ok(Self::Context),
            "glossary" => Ok(Self::Glossary),
            "runbook" => Ok(Self::Runbook),
            other => Err(format!(
                "invalid doc type `{other}`; expected one of: {}",
                DOC_TYPES.join(", ")
            )),
        }
    }
}

impl Serialize for DocType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DocType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactRef {
    Task(String),
    Learning(String),
    Friction(String),
    Adr(String),
}

impl ArtifactRef {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Task(value)
            | Self::Learning(value)
            | Self::Friction(value)
            | Self::Adr(value) => value,
        }
    }
}

impl Serialize for ArtifactRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ArtifactRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        parse_artifact_ref(&raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocFrontmatter {
    #[serde(rename = "type")]
    pub doc_type: DocType,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_features: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocRecord {
    pub path: String,
    #[serde(flatten)]
    pub frontmatter: DocFrontmatter,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocShow {
    pub path: String,
    pub frontmatter: DocFrontmatter,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocSearchResult {
    #[serde(flatten)]
    pub record: DocRecord,
    pub score: usize,
    pub matched_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocAddOutcome {
    pub path: String,
    pub added: bool,
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocMigrationReport {
    pub dry_run: bool,
    pub changed: Vec<DocMigrationChange>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocMigrationChange {
    pub path: String,
    pub diff: String,
}

#[derive(Debug, Deserialize)]
struct RawDocFrontmatter {
    #[serde(rename = "type")]
    doc_type: Option<DocType>,
    summary: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    related_features: Vec<String>,
    #[serde(default)]
    related_artifacts: Vec<ArtifactRef>,
}

#[derive(Debug)]
struct ParsedDoc {
    frontmatter: DocFrontmatter,
    body: String,
}

#[derive(Debug)]
struct FrontmatterBlock<'a> {
    raw: &'a str,
    body: &'a str,
}

#[derive(Debug, Deserialize)]
struct DocsConfigFile {
    docs: Option<DocsConfigSection>,
}

#[derive(Debug, Deserialize)]
struct DocsConfigSection {
    roots: Option<Vec<String>>,
}

impl OrbitRuntime {
    pub fn docs_roots(&self) -> Result<Vec<String>, OrbitError> {
        read_docs_roots_from_config_path(&self.config_path())
    }

    pub fn list_docs(
        &self,
        doc_type: Option<DocType>,
        tag: Option<&str>,
    ) -> Result<Vec<DocRecord>, OrbitError> {
        let mut records = walk_docs_roots(&self.paths().repo_root, &self.docs_roots()?)?;
        if let Some(doc_type) = doc_type {
            records.retain(|record| record.frontmatter.doc_type == doc_type);
        }
        if let Some(tag) = tag.map(|value| value.trim().to_ascii_lowercase())
            && !tag.is_empty()
        {
            records.retain(|record| {
                record
                    .frontmatter
                    .tags
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(&tag))
            });
        }
        Ok(records)
    }

    pub fn show_doc(&self, path: &str) -> Result<DocShow, OrbitError> {
        show_doc(&self.paths().repo_root, &self.docs_roots()?, path)
    }

    pub fn search_docs(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<DocSearchResult>, OrbitError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(OrbitError::InvalidInput(
                "docs search query must not be empty".to_string(),
            ));
        }
        let limit = limit.unwrap_or(20);
        let query_lower = query.to_ascii_lowercase();
        let mut scored = self
            .list_docs(None, None)?
            .into_iter()
            .filter_map(|record| score_doc_record(record, &query_lower))
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.record.path.cmp(&right.record.path))
        });
        scored.truncate(limit);
        Ok(scored)
    }

    pub fn add_docs_root(&self, path: &str) -> Result<DocAddOutcome, OrbitError> {
        add_docs_root(&self.paths().repo_root, &self.config_path(), path)
    }

    pub fn reindex_docs(&self) -> Result<String, OrbitError> {
        Ok("indexer is walk-on-demand; nothing to do.".to_string())
    }

    pub fn migrate_docs(&self, dry_run: bool) -> Result<DocMigrationReport, OrbitError> {
        migrate_docs(&self.paths().repo_root, dry_run)
    }
}

pub fn parse_docs_roots_from_config_toml(raw: &str) -> Result<Vec<String>, OrbitError> {
    if raw.trim().is_empty() {
        return Ok(default_doc_roots());
    }
    let parsed = toml::from_str::<DocsConfigFile>(raw).map_err(|error| {
        OrbitError::InvalidInput(format!("invalid docs config in config.toml: {error}"))
    })?;
    Ok(parsed
        .docs
        .and_then(|section| section.roots)
        .unwrap_or_else(default_doc_roots))
}

fn read_docs_roots_from_config_path(path: &Path) -> Result<Vec<String>, OrbitError> {
    if !path.exists() {
        return Ok(default_doc_roots());
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
    parse_docs_roots_from_config_toml(&raw)
}

fn default_doc_roots() -> Vec<String> {
    vec![DEFAULT_DOC_ROOT.to_string()]
}

pub fn parse_doc_frontmatter_strict(path: &Path, raw: &str) -> Result<DocFrontmatter, OrbitError> {
    parse_doc_strict(path, raw).map(|parsed| parsed.frontmatter)
}

fn parse_doc_strict(path: &Path, raw: &str) -> Result<ParsedDoc, OrbitError> {
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

fn parse_doc_tolerant(repo_relative: &Path, absolute_path: &Path, raw: &str) -> ParsedDoc {
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

fn split_frontmatter(raw: &str) -> Result<Option<FrontmatterBlock<'_>>, String> {
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

fn infer_frontmatter(repo_relative: &Path, body: &str) -> DocFrontmatter {
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
    if components.iter().any(|component| *component == "runbooks") {
        return (DocType::Runbook, Vec::new());
    }
    if repo_relative
        .file_stem()
        .and_then(|value| value.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("glossary"))
        || components.iter().any(|component| *component == "glossary")
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

pub fn walk_docs_roots(repo_root: &Path, roots: &[String]) -> Result<Vec<DocRecord>, OrbitError> {
    let mut records = Vec::new();
    for root in roots {
        for path in expand_root(repo_root, root)? {
            if path_is_or_contains_dot_orbit(repo_root, &path) {
                continue;
            }
            if path.is_file() {
                maybe_push_doc(repo_root, &path, &mut records)?;
            } else if path.is_dir() {
                walk_dir(repo_root, &path, &mut records)?;
            }
        }
    }
    records.sort_by(|left, right| left.path.cmp(&right.path));
    records.dedup_by(|left, right| left.path == right.path);
    Ok(records)
}

fn expand_root(repo_root: &Path, root: &str) -> Result<Vec<PathBuf>, OrbitError> {
    let trimmed = root.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let root_path = Path::new(trimmed);
    let absolute = if root_path.is_absolute() {
        root_path.to_path_buf()
    } else {
        repo_root.join(root_path)
    };
    if !trimmed.contains('*') {
        if absolute.exists() {
            return Ok(vec![absolute]);
        }
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    expand_wildcard_segments(repo_root, Path::new(trimmed), &mut out)?;
    Ok(out)
}

fn expand_wildcard_segments(
    base: &Path,
    pattern: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<(), OrbitError> {
    fn rec(base: &Path, parts: &[String], out: &mut Vec<PathBuf>) -> Result<(), OrbitError> {
        if parts.is_empty() {
            if base.exists() {
                out.push(base.to_path_buf());
            }
            return Ok(());
        }
        let head = &parts[0];
        let tail = &parts[1..];
        if head == "*" {
            if !base.is_dir() {
                return Ok(());
            }
            let entries = fs::read_dir(base)
                .map_err(|error| OrbitError::Io(format!("read {}: {error}", base.display())))?;
            for entry in entries {
                let entry = entry.map_err(|error| OrbitError::Io(error.to_string()))?;
                if entry
                    .file_type()
                    .map_err(|error| OrbitError::Io(error.to_string()))?
                    .is_dir()
                {
                    rec(&entry.path(), tail, out)?;
                }
            }
            return Ok(());
        }
        rec(&base.join(head), tail, out)
    }

    let parts = pattern
        .components()
        .filter_map(component_str)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    rec(base, &parts, out)
}

fn walk_dir(repo_root: &Path, dir: &Path, records: &mut Vec<DocRecord>) -> Result<(), OrbitError> {
    if should_skip_dir(repo_root, dir) {
        return Ok(());
    }
    let mut entries = fs::read_dir(dir)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", dir.display())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| OrbitError::Io(error.to_string()))?;
        if file_type.is_dir() {
            walk_dir(repo_root, &path, records)?;
        } else if file_type.is_file() {
            maybe_push_doc(repo_root, &path, records)?;
        }
    }
    Ok(())
}

fn maybe_push_doc(
    repo_root: &Path,
    path: &Path,
    records: &mut Vec<DocRecord>,
) -> Result<(), OrbitError> {
    if path.extension().and_then(|value| value.to_str()) != Some("md") {
        return Ok(());
    }
    if path_is_or_contains_dot_orbit(repo_root, path) {
        return Ok(());
    }
    let relative = repo_relative_path(repo_root, path)?;
    if is_git_ignored(repo_root, &relative) {
        return Ok(());
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
    let parsed = parse_doc_tolerant(&relative, path, &raw);
    records.push(DocRecord {
        path: path_to_slash_string(&relative),
        frontmatter: parsed.frontmatter,
    });
    Ok(())
}

fn should_skip_dir(repo_root: &Path, dir: &Path) -> bool {
    let Some(name) = dir.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    if matches!(name, ".orbit" | ".git" | "node_modules" | "target") {
        return true;
    }
    path_is_or_contains_dot_orbit(repo_root, dir)
}

fn path_is_or_contains_dot_orbit(repo_root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(repo_root).unwrap_or(path);
    relative
        .components()
        .any(|component| matches!(component, Component::Normal(value) if value == ".orbit"))
}

fn is_git_ignored(repo_root: &Path, relative: &Path) -> bool {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("check-ignore")
        .arg("-q")
        .arg("--")
        .arg(relative)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    matches!(status, Ok(status) if status.success())
}

fn show_doc(repo_root: &Path, roots: &[String], requested: &str) -> Result<DocShow, OrbitError> {
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
    let raw = fs::read_to_string(&absolute)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", absolute.display())))?;
    let parsed = parse_doc_tolerant(&relative, &absolute, &raw);
    Ok(DocShow {
        path: path_to_slash_string(&relative),
        frontmatter: parsed.frontmatter,
        body: parsed.body,
    })
}

fn path_is_under_configured_roots(
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

fn score_doc_record(record: DocRecord, query_lower: &str) -> Option<DocSearchResult> {
    let mut score = 0usize;
    let mut matched_by = Vec::new();
    let summary = record.frontmatter.summary.to_ascii_lowercase();
    if summary.contains(query_lower) {
        score += 80 + query_lower.len();
        matched_by.push("summary".to_string());
    }
    let doc_type = record.frontmatter.doc_type.as_str();
    if doc_type.contains(query_lower) {
        score += 30;
        matched_by.push(format!("type:{doc_type}"));
    }
    for tag in &record.frontmatter.tags {
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

fn add_docs_root(
    repo_root: &Path,
    config_path: &Path,
    path: &str,
) -> Result<DocAddOutcome, OrbitError> {
    let normalized = normalize_docs_root_arg(repo_root, path)?;
    let raw = if config_path.exists() {
        fs::read_to_string(config_path)
            .map_err(|error| OrbitError::Io(format!("read {}: {error}", config_path.display())))?
    } else {
        String::new()
    };
    let mut roots = parse_docs_roots_from_config_toml(&raw)?;
    if roots_equal_contains(&roots, &normalized) {
        return Ok(DocAddOutcome {
            path: normalized,
            added: false,
            roots,
        });
    }
    roots.push(normalized.clone());
    write_docs_roots_to_config(config_path, &raw, &roots)?;
    Ok(DocAddOutcome {
        path: normalized,
        added: true,
        roots,
    })
}

fn normalize_docs_root_arg(repo_root: &Path, raw: &str) -> Result<String, OrbitError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(
            "docs root path must not be empty".to_string(),
        ));
    }
    let input = Path::new(trimmed);
    let absolute = if input.is_absolute() {
        input.to_path_buf()
    } else {
        repo_root.join(input)
    };
    if !absolute.exists() {
        return Err(OrbitError::InvalidInput(format!(
            "docs root path does not exist: {trimmed}"
        )));
    }
    let canonical_repo = repo_root.canonicalize().map_err(|error| {
        OrbitError::Io(format!("canonicalize {}: {error}", repo_root.display()))
    })?;
    let canonical = absolute
        .canonicalize()
        .map_err(|error| OrbitError::Io(format!("canonicalize {}: {error}", absolute.display())))?;
    let orbit_dir = canonical_repo.join(".orbit");
    if canonical.starts_with(&orbit_dir) {
        return Err(OrbitError::InvalidInput(
            "orbit docs add refuses paths under .orbit/".to_string(),
        ));
    }
    let relative = canonical.strip_prefix(&canonical_repo).map_err(|_| {
        OrbitError::InvalidInput(format!(
            "docs root path must stay inside the workspace root: {trimmed}"
        ))
    })?;
    let mut normalized = path_to_slash_string(relative);
    if canonical.is_dir() && !normalized.ends_with('/') {
        normalized.push('/');
    }
    Ok(normalized)
}

fn roots_equal_contains(roots: &[String], candidate: &str) -> bool {
    let candidate = comparable_root(candidate);
    roots
        .iter()
        .any(|root| comparable_root(root.as_str()) == candidate)
}

fn comparable_root(raw: &str) -> String {
    raw.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn write_docs_roots_to_config(
    config_path: &Path,
    raw: &str,
    roots: &[String],
) -> Result<(), OrbitError> {
    let rendered = if raw.trim().is_empty() || !raw.contains("[docs]") {
        let mut out = raw.trim_end().to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str("[docs]\nroots = ");
        out.push_str(&json!(roots).to_string());
        out.push('\n');
        out
    } else {
        let mut value = raw.parse::<toml::Value>().map_err(|error| {
            OrbitError::InvalidInput(format!(
                "invalid config.toml while updating [docs].roots: {error}"
            ))
        })?;
        let table = value.as_table_mut().ok_or_else(|| {
            OrbitError::InvalidInput("config.toml must be a TOML table".to_string())
        })?;
        let docs = table
            .entry("docs".to_string())
            .or_insert_with(|| toml::Value::Table(Default::default()));
        let docs_table = docs.as_table_mut().ok_or_else(|| {
            OrbitError::InvalidInput("[docs] config must be a TOML table".to_string())
        })?;
        docs_table.insert(
            "roots".to_string(),
            toml::Value::Array(roots.iter().cloned().map(toml::Value::String).collect()),
        );
        toml::to_string_pretty(&value)
            .map_err(|error| OrbitError::Execution(format!("serialize config.toml: {error}")))?
    };
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| OrbitError::Io(format!("create {}: {error}", parent.display())))?;
    }
    fs::write(config_path, rendered)
        .map_err(|error| OrbitError::Io(format!("write {}: {error}", config_path.display())))
}

fn migrate_docs(repo_root: &Path, dry_run: bool) -> Result<DocMigrationReport, OrbitError> {
    let mut candidates = Vec::new();
    collect_migration_candidates(&repo_root.join("docs/design"), 2, &mut candidates)?;
    collect_migration_candidates(&repo_root.join("docs/design-patterns"), 1, &mut candidates)?;
    candidates.sort();
    let mut changed = Vec::new();
    for path in candidates {
        if path_is_or_contains_dot_orbit(repo_root, &path) {
            continue;
        }
        let relative = repo_relative_path(repo_root, &path)?;
        let raw = fs::read_to_string(&path)
            .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
        let Some(updated) = migrate_doc_content(&relative, &path, &raw)? else {
            continue;
        };
        let diff = migration_diff(&path_to_slash_string(&relative), &raw, &updated);
        if !dry_run {
            fs::write(&path, &updated)
                .map_err(|error| OrbitError::Io(format!("write {}: {error}", path.display())))?;
        }
        changed.push(DocMigrationChange {
            path: path_to_slash_string(&relative),
            diff,
        });
    }
    Ok(DocMigrationReport { dry_run, changed })
}

fn collect_migration_candidates(
    root: &Path,
    relative_depth: usize,
    out: &mut Vec<PathBuf>,
) -> Result<(), OrbitError> {
    if !root.exists() {
        return Ok(());
    }
    fn rec(
        root: &Path,
        dir: &Path,
        relative_depth: usize,
        out: &mut Vec<PathBuf>,
    ) -> Result<(), OrbitError> {
        let mut entries = fs::read_dir(dir)
            .map_err(|error| OrbitError::Io(format!("read {}: {error}", dir.display())))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| OrbitError::Io(error.to_string()))?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| OrbitError::Io(error.to_string()))?;
            if file_type.is_dir() {
                rec(root, &path, relative_depth, out)?;
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("md") {
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            if relative.components().count() == relative_depth {
                out.push(path);
            }
        }
        Ok(())
    }
    rec(root, root, relative_depth, out)
}

fn migrate_doc_content(
    relative: &Path,
    path: &Path,
    raw: &str,
) -> Result<Option<String>, OrbitError> {
    if parse_doc_strict(path, raw).is_ok() {
        return Ok(None);
    }
    let block = split_frontmatter(raw).map_err(|message| {
        OrbitError::InvalidInput(format!(
            "invalid frontmatter in {}: {message}",
            path.display()
        ))
    })?;
    let body = block.as_ref().map(|block| block.body).unwrap_or(raw);
    let inferred = infer_frontmatter(relative, body);
    let updated = match block {
        Some(block) => update_existing_frontmatter(block.raw, body, &inferred),
        None => {
            let mut output = render_frontmatter_block(&inferred);
            output.push_str(raw);
            output
        }
    };
    if updated == raw {
        return Ok(None);
    }
    Ok(Some(updated))
}

fn update_existing_frontmatter(existing: &str, body: &str, inferred: &DocFrontmatter) -> String {
    let mut lines = existing.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    upsert_yaml_scalar(&mut lines, "type", inferred.doc_type.as_str());
    upsert_yaml_scalar(
        &mut lines,
        "summary",
        &yaml_inline_string(&inferred.summary),
    );
    if !inferred.tags.is_empty() && !has_yaml_key(&lines, "tags") {
        lines.push(format!("tags: {}", json!(inferred.tags)));
    }
    let mut output = String::from("---\n");
    output.push_str(&lines.join("\n"));
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("---\n");
    output.push_str(body);
    output
}

fn upsert_yaml_scalar(lines: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{key}:");
    for line in lines.iter_mut() {
        if line.trim_start().starts_with(&prefix) {
            *line = format!("{key}: {value}");
            return;
        }
    }
    lines.insert(0, format!("{key}: {value}"));
}

fn has_yaml_key(lines: &[String], key: &str) -> bool {
    let prefix = format!("{key}:");
    lines
        .iter()
        .any(|line| line.trim_start().starts_with(&prefix))
}

fn render_frontmatter_block(frontmatter: &DocFrontmatter) -> String {
    let mut output = String::from("---\n");
    output.push_str(&format!("type: {}\n", frontmatter.doc_type));
    output.push_str(&format!(
        "summary: {}\n",
        yaml_inline_string(&frontmatter.summary)
    ));
    if !frontmatter.tags.is_empty() {
        output.push_str(&format!("tags: {}\n", json!(frontmatter.tags)));
    }
    if !frontmatter.paths.is_empty() {
        output.push_str(&format!("paths: {}\n", json!(frontmatter.paths)));
    }
    if !frontmatter.related_features.is_empty() {
        output.push_str(&format!(
            "related_features: {}\n",
            json!(frontmatter.related_features)
        ));
    }
    if !frontmatter.related_artifacts.is_empty() {
        output.push_str(&format!(
            "related_artifacts: {}\n",
            json!(frontmatter.related_artifacts)
        ));
    }
    output.push_str("---\n");
    output
}

fn yaml_inline_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn migration_diff(path: &str, before: &str, after: &str) -> String {
    let before_head = before.lines().take(12).collect::<Vec<_>>().join("\n");
    let after_head = after.lines().take(16).collect::<Vec<_>>().join("\n");
    format!("--- {path}\n+++ {path}\n@@\n-{before_head}\n+{after_head}\n")
}

fn parse_artifact_ref(raw: &str) -> Result<ArtifactRef, String> {
    let trimmed = raw.trim();
    if is_task_ref(trimmed) {
        return Ok(ArtifactRef::Task(trimmed.to_string()));
    }
    if is_learning_ref(trimmed) {
        return Ok(ArtifactRef::Learning(trimmed.to_string()));
    }
    if is_friction_ref(trimmed) {
        return Ok(ArtifactRef::Friction(trimmed.to_string()));
    }
    if is_adr_ref(trimmed) {
        return Ok(ArtifactRef::Adr(trimmed.to_string()));
    }
    Err(format!(
        "unknown related_artifacts reference `{trimmed}`; expected ORB-NNNNN, LYYYYMMDD-N, FYYYY-MM-NNN, or ADR-NNNN"
    ))
}

fn is_task_ref(value: &str) -> bool {
    value.len() == 9
        && value.starts_with("ORB-")
        && value[4..].chars().all(|ch| ch.is_ascii_digit())
}

fn is_learning_ref(value: &str) -> bool {
    let Some(rest) = value.strip_prefix('L') else {
        return false;
    };
    let Some((date, ordinal)) = rest.split_once('-') else {
        return false;
    };
    date.len() == 8
        && date.chars().all(|ch| ch.is_ascii_digit())
        && !ordinal.is_empty()
        && ordinal.chars().all(|ch| ch.is_ascii_digit())
}

fn is_friction_ref(value: &str) -> bool {
    let Some(rest) = value.strip_prefix('F') else {
        return false;
    };
    let parts = rest.split('-').collect::<Vec<_>>();
    parts.len() == 3
        && parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 3
        && parts
            .iter()
            .all(|part| part.chars().all(|ch| ch.is_ascii_digit()))
}

fn is_adr_ref(value: &str) -> bool {
    value.len() == 8
        && value.starts_with("ADR-")
        && value[4..].chars().all(|ch| ch.is_ascii_digit())
}

fn repo_relative_path(repo_root: &Path, path: &Path) -> Result<PathBuf, OrbitError> {
    if let Ok(relative) = path.strip_prefix(repo_root) {
        return Ok(relative.to_path_buf());
    }
    let canonical_repo = repo_root.canonicalize().map_err(|error| {
        OrbitError::Io(format!("canonicalize {}: {error}", repo_root.display()))
    })?;
    let canonical_path = path
        .canonicalize()
        .map_err(|error| OrbitError::Io(format!("canonicalize {}: {error}", path.display())))?;
    canonical_path
        .strip_prefix(canonical_repo)
        .map(Path::to_path_buf)
        .map_err(|_| {
            OrbitError::InvalidInput(format!(
                "path is outside workspace root: {}",
                path.display()
            ))
        })
}

fn path_to_slash_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn component_str(component: Component<'_>) -> Option<&str> {
    match component {
        Component::Normal(value) => value.to_str(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn parse_frontmatter(raw: &str) -> Result<DocFrontmatter, OrbitError> {
        parse_doc_frontmatter_strict(Path::new("docs/example.md"), raw)
    }

    #[test]
    fn strict_frontmatter_accepts_locked_schema() {
        let parsed = parse_frontmatter(
            "---\ntype: design\nsummary: Hook rewrite design\ntags: [hook, audit]\nrelated_artifacts: [ORB-00160, ADR-0168, L20260514-3, F2026-05-001]\n---\n# Body\n",
        )
        .expect("valid frontmatter");
        assert_eq!(parsed.doc_type, DocType::Design);
        assert_eq!(parsed.summary, "Hook rewrite design");
        assert_eq!(parsed.tags, vec!["hook", "audit"]);
        assert_eq!(parsed.related_artifacts.len(), 4);
    }

    #[test]
    fn strict_frontmatter_rejects_missing_required_fields() {
        let missing_type =
            parse_frontmatter("---\nsummary: A doc\n---\nbody\n").expect_err("missing type");
        assert!(
            missing_type
                .to_string()
                .contains("missing required field `type`")
        );
        let missing_summary =
            parse_frontmatter("---\ntype: design\n---\nbody\n").expect_err("missing summary");
        assert!(
            missing_summary
                .to_string()
                .contains("missing required field `summary`")
        );
    }

    #[test]
    fn strict_frontmatter_rejects_unknown_artifact_prefix() {
        let error = parse_frontmatter(
            "---\ntype: design\nsummary: A doc\nrelated_artifacts: [XYZ-1]\n---\nbody\n",
        )
        .expect_err("unknown artifact prefix");
        assert!(error.to_string().contains("unknown related_artifacts"));
    }

    #[test]
    fn tolerant_frontmatter_infers_legacy_design_doc() {
        let parsed = parse_doc_tolerant(
            Path::new("docs/design/hook-rewrite/4_decisions.md"),
            Path::new("docs/design/hook-rewrite/4_decisions.md"),
            "# Decisions\n\nBody\n",
        );
        assert_eq!(parsed.frontmatter.doc_type, DocType::Design);
        assert_eq!(parsed.frontmatter.tags, vec!["hook-rewrite"]);
        assert_eq!(parsed.frontmatter.summary, "Decisions");
    }

    #[test]
    fn tolerant_frontmatter_infers_design_pattern_doc() {
        let parsed = parse_doc_tolerant(
            Path::new("docs/design-patterns/error_translation.md"),
            Path::new("docs/design-patterns/error_translation.md"),
            "# Crate-Boundary Error Translation\n",
        );
        assert_eq!(parsed.frontmatter.doc_type, DocType::Pattern);
        assert_eq!(
            parsed.frontmatter.summary,
            "Crate-Boundary Error Translation"
        );
    }

    #[test]
    fn malformed_yaml_errors_in_strict_and_falls_back_in_tolerant() {
        let raw = "---\ntype: [\nsummary: bad\n---\n# Fallback\n";
        assert!(parse_frontmatter(raw).is_err());
        let parsed = parse_doc_tolerant(Path::new("docs/context/bad.md"), Path::new("bad.md"), raw);
        assert_eq!(parsed.frontmatter.doc_type, DocType::Context);
    }

    #[test]
    fn walker_skips_dot_orbit_even_when_root_points_above_it() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("docs")).expect("docs dir");
        fs::write(
            root.join("docs/good.md"),
            "---\ntype: context\nsummary: Good doc\n---\nbody\n",
        )
        .expect("write good");
        fs::create_dir_all(root.join(".orbit/adrs/ADR-0001")).expect("adr dir");
        fs::write(root.join(".orbit/adrs/ADR-0001/body.md"), "# ADR\n").expect("write adr");

        let records = walk_docs_roots(root, &[".".to_string()]).expect("walk docs");
        assert_eq!(
            records
                .iter()
                .map(|record| record.path.as_str())
                .collect::<Vec<_>>(),
            vec!["docs/good.md"]
        );
    }

    #[test]
    fn config_roots_default_and_parse_explicit_values() {
        assert_eq!(
            parse_docs_roots_from_config_toml("").unwrap(),
            vec!["docs/"]
        );
        assert_eq!(
            parse_docs_roots_from_config_toml("[docs]\nroots = [\"docs/\", \"apps/*/docs/\"]\n")
                .unwrap(),
            vec!["docs/", "apps/*/docs/"]
        );
    }

    #[test]
    fn migrate_adds_locked_fields_to_legacy_frontmatter() {
        let raw = "---\ntitle: Example\nowner: codex\n---\n\n# Example\n";
        let updated = migrate_doc_content(
            Path::new("docs/design/sample/1_overview.md"),
            Path::new("docs/design/sample/1_overview.md"),
            raw,
        )
        .expect("migrate")
        .expect("changed");
        let parsed = parse_doc_frontmatter_strict(Path::new("doc.md"), &updated)
            .expect("valid locked schema");
        assert_eq!(parsed.doc_type, DocType::Design);
        assert_eq!(parsed.tags, vec!["sample"]);
        assert!(
            migrate_doc_content(
                Path::new("docs/design/sample/1_overview.md"),
                Path::new("doc.md"),
                &updated
            )
            .unwrap()
            .is_none()
        );
    }
}
