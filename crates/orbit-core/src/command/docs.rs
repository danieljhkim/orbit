use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;

use orbit_common::types::{Adr, AdrStatus, OrbitError, Task};
use orbit_common::utility::glob::{match_glob, normalize_glob_path};
use orbit_common::utility::selector::anchor_path;
pub use orbit_search::{AdrSearchResult, DocSearchResult, SearchResult};
use orbit_search::{
    AdrSearchSource, DocSearchSource, score_adr_record, score_doc_record, sort_search_results,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::json;

use crate::OrbitRuntime;

const DEFAULT_DOC_ROOT: &str = "docs/";
const DOC_TYPES: &[&str] = &["design", "pattern", "context", "glossary", "runbook"];
const DEFAULT_RELATED_DOC_LIMIT: usize = 5;

#[cfg(test)]
thread_local! {
    static GIT_CHECK_IGNORE_INVOCATIONS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn record_git_check_ignore_invocation() {
    GIT_CHECK_IGNORE_INVOCATIONS.with(|calls| calls.set(calls.get() + 1));
}

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

/// Related-doc projection emitted by `task show --with-context`.
///
/// JSON schema:
/// `{"path": string, "type": string, "summary": string, "excerpt": string, "matched_by": string[]}`.
/// The `type` value is one of `design`, `pattern`, `context`, `glossary`, or
/// `runbook`. `matched_by` contains stable `path:<glob>` and `feature:<slug>`
/// markers explaining why the doc was selected.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TaskRelatedDoc {
    pub path: String,
    #[serde(rename = "type")]
    pub doc_type: DocType,
    pub summary: String,
    pub excerpt: String,
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
        include_superseded: bool,
    ) -> Result<Vec<SearchResult>, OrbitError> {
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
            .map(doc_search_source)
            .filter_map(|record| score_doc_record(record, &query_lower))
            .map(SearchResult::Doc)
            .collect::<Vec<_>>();
        scored.extend(
            self.stores()
                .adrs()
                .list()?
                .into_iter()
                .filter(|adr| adr_status_in_docs_search(adr.status, include_superseded))
                .map(adr_search_source)
                .filter_map(|adr| score_adr_record(adr, &query_lower))
                .map(SearchResult::Adr),
        );
        sort_search_results(&mut scored);
        scored.truncate(limit);
        Ok(scored)
    }

    pub fn related_docs_for_task(
        &self,
        task: &Task,
        limit: Option<usize>,
    ) -> Result<Vec<TaskRelatedDoc>, OrbitError> {
        let roots = read_task_context_docs_roots_from_config_path(&self.config_path())?;
        if roots.is_empty() {
            return Ok(Vec::new());
        }
        // Tasks do not yet have a first-class `related_features` field, so the
        // agent-facing feature join uses normalized task tags as the feature
        // selectors until that storage field exists.
        related_docs_for_context(
            &self.paths().repo_root,
            &roots,
            &task.context_files,
            &task.tags,
            limit,
        )
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

fn read_task_context_docs_roots_from_config_path(path: &Path) -> Result<Vec<String>, OrbitError> {
    if !path.exists() {
        return Ok(default_doc_roots());
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
    parse_task_context_docs_roots_from_config_toml(&raw)
}

fn parse_task_context_docs_roots_from_config_toml(raw: &str) -> Result<Vec<String>, OrbitError> {
    if raw.trim().is_empty() {
        return Ok(default_doc_roots());
    }
    let parsed = toml::from_str::<DocsConfigFile>(raw).map_err(|error| {
        OrbitError::InvalidInput(format!("invalid docs config in config.toml: {error}"))
    })?;
    Ok(match parsed.docs {
        Some(section) => section.roots.unwrap_or_default(),
        None => default_doc_roots(),
    })
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

pub fn walk_docs_roots(repo_root: &Path, roots: &[String]) -> Result<Vec<DocRecord>, OrbitError> {
    let mut candidates = Vec::new();
    for root in roots {
        for path in expand_root(repo_root, root)? {
            if path_is_or_contains_dot_orbit(repo_root, &path) {
                continue;
            }
            if path.is_file() {
                maybe_push_doc_candidate(repo_root, &path, &mut candidates)?;
            } else if path.is_dir() {
                walk_dir(repo_root, &path, &mut candidates)?;
            }
        }
    }
    candidates.sort();
    candidates.dedup();

    let ignored = git_ignored_paths(repo_root, &candidates);
    let mut records = Vec::new();
    for relative in candidates {
        if ignored.contains(&relative) {
            continue;
        }
        let path = repo_root.join(&relative);
        let raw = fs::read_to_string(&path)
            .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
        let parsed = parse_doc_tolerant(&relative, &path, &raw);
        records.push(DocRecord {
            path: path_to_slash_string(&relative),
            frontmatter: parsed.frontmatter,
        });
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

fn walk_dir(repo_root: &Path, dir: &Path, candidates: &mut Vec<PathBuf>) -> Result<(), OrbitError> {
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
            walk_dir(repo_root, &path, candidates)?;
        } else if file_type.is_file() {
            maybe_push_doc_candidate(repo_root, &path, candidates)?;
        }
    }
    Ok(())
}

fn maybe_push_doc_candidate(
    repo_root: &Path,
    path: &Path,
    candidates: &mut Vec<PathBuf>,
) -> Result<(), OrbitError> {
    if path.extension().and_then(|value| value.to_str()) != Some("md") {
        return Ok(());
    }
    if path_is_or_contains_dot_orbit(repo_root, path) {
        return Ok(());
    }
    let relative = repo_relative_path(repo_root, path)?;
    candidates.push(relative);
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

fn git_ignored_paths(repo_root: &Path, relatives: &[PathBuf]) -> HashSet<PathBuf> {
    let mut ignored = HashSet::new();
    if relatives.is_empty() {
        return ignored;
    }
    #[cfg(test)]
    record_git_check_ignore_invocation();
    let mut child = match Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("check-ignore")
        .arg("-z")
        .arg("--stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return ignored,
    };
    let mut wrote_all = true;
    if let Some(mut stdin) = child.stdin.take() {
        for relative in relatives {
            let path = path_to_slash_string(relative);
            if stdin.write_all(path.as_bytes()).is_err() || stdin.write_all(b"\0").is_err() {
                wrote_all = false;
                break;
            }
        }
    }
    if !wrote_all {
        let _ = child.wait();
        return ignored;
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(_) => return ignored,
    };
    if !output.status.success() {
        return ignored;
    }
    for raw_path in output.stdout.split(|byte| *byte == 0) {
        if raw_path.is_empty() {
            continue;
        }
        ignored.insert(PathBuf::from(String::from_utf8_lossy(raw_path).to_string()));
    }
    ignored
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

fn adr_status_in_docs_search(status: AdrStatus, include_superseded: bool) -> bool {
    matches!(status, AdrStatus::Proposed | AdrStatus::Accepted)
        || (include_superseded && status == AdrStatus::Superseded)
}

fn doc_search_source(record: DocRecord) -> DocSearchSource {
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

fn adr_search_source(adr: Adr) -> AdrSearchSource {
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

fn adr_body_search_path(status: AdrStatus, id: &str) -> PathBuf {
    PathBuf::from(".orbit")
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

fn related_docs_for_context(
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
        Some(block) => update_existing_frontmatter(block.raw, body, &inferred)?,
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

fn update_existing_frontmatter(
    existing: &str,
    body: &str,
    inferred: &DocFrontmatter,
) -> Result<String, OrbitError> {
    let mut value = if existing.trim().is_empty() {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str::<serde_yaml::Value>(existing).map_err(|error| {
            OrbitError::InvalidInput(format!("invalid frontmatter YAML while migrating: {error}"))
        })?
    };
    if matches!(value, serde_yaml::Value::Null) {
        value = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }
    let serde_yaml::Value::Mapping(mapping) = &mut value else {
        return Err(OrbitError::InvalidInput(
            "frontmatter YAML must be a mapping to migrate".to_string(),
        ));
    };
    mapping.insert(
        serde_yaml::Value::String("type".to_string()),
        serde_yaml::Value::String(inferred.doc_type.as_str().to_string()),
    );
    mapping.insert(
        serde_yaml::Value::String("summary".to_string()),
        serde_yaml::Value::String(inferred.summary.clone()),
    );
    let tags_key = serde_yaml::Value::String("tags".to_string());
    if !inferred.tags.is_empty() && !mapping.contains_key(&tags_key) {
        mapping.insert(
            tags_key,
            serde_yaml::Value::Sequence(
                inferred
                    .tags
                    .iter()
                    .cloned()
                    .map(serde_yaml::Value::String)
                    .collect(),
            ),
        );
    }
    let mut rendered = serde_yaml::to_string(&value)
        .map_err(|error| OrbitError::Execution(format!("serialize frontmatter YAML: {error}")))?;
    if let Some(stripped) = rendered.strip_prefix("---\n") {
        rendered = stripped.to_string();
    }
    if let Some(stripped) = rendered.strip_suffix("...\n") {
        rendered = stripped.to_string();
    }
    let mut output = String::from("---\n");
    output.push_str(&rendered);
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("---\n");
    output.push_str(body);
    Ok(output)
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
    let before_lines = diff_lines(before);
    let after_lines = diff_lines(after);
    let old_start = if before_lines.is_empty() { 0 } else { 1 };
    let new_start = if after_lines.is_empty() { 0 } else { 1 };
    let mut output = format!(
        "--- {path}\n+++ {path}\n@@ -{old_start},{} +{new_start},{} @@\n",
        before_lines.len(),
        after_lines.len()
    );
    for op in line_diff(&before_lines, &after_lines) {
        match op {
            DiffOp::Equal(line) => push_diff_line(&mut output, ' ', line),
            DiffOp::Delete(line) => push_diff_line(&mut output, '-', line),
            DiffOp::Insert(line) => push_diff_line(&mut output, '+', line),
        }
    }
    output
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct DiffLine<'a> {
    text: &'a str,
    has_newline: bool,
}

enum DiffOp<'a> {
    Equal(DiffLine<'a>),
    Delete(DiffLine<'a>),
    Insert(DiffLine<'a>),
}

fn diff_lines(raw: &str) -> Vec<DiffLine<'_>> {
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split_inclusive('\n')
        .map(|line| match line.strip_suffix('\n') {
            Some(without_newline) => DiffLine {
                text: without_newline
                    .strip_suffix('\r')
                    .unwrap_or(without_newline),
                has_newline: true,
            },
            None => DiffLine {
                text: line,
                has_newline: false,
            },
        })
        .collect()
}

fn line_diff<'a>(before: &[DiffLine<'a>], after: &[DiffLine<'a>]) -> Vec<DiffOp<'a>> {
    let mut lcs = vec![vec![0usize; after.len() + 1]; before.len() + 1];
    for before_index in (0..before.len()).rev() {
        for after_index in (0..after.len()).rev() {
            lcs[before_index][after_index] = if before[before_index] == after[after_index] {
                lcs[before_index + 1][after_index + 1] + 1
            } else {
                lcs[before_index + 1][after_index].max(lcs[before_index][after_index + 1])
            };
        }
    }
    let mut ops = Vec::new();
    let mut before_index = 0;
    let mut after_index = 0;
    while before_index < before.len() && after_index < after.len() {
        if before[before_index] == after[after_index] {
            ops.push(DiffOp::Equal(before[before_index]));
            before_index += 1;
            after_index += 1;
        } else if lcs[before_index + 1][after_index] >= lcs[before_index][after_index + 1] {
            ops.push(DiffOp::Delete(before[before_index]));
            before_index += 1;
        } else {
            ops.push(DiffOp::Insert(after[after_index]));
            after_index += 1;
        }
    }
    while before_index < before.len() {
        ops.push(DiffOp::Delete(before[before_index]));
        before_index += 1;
    }
    while after_index < after.len() {
        ops.push(DiffOp::Insert(after[after_index]));
        after_index += 1;
    }
    ops
}

fn push_diff_line(output: &mut String, prefix: char, line: DiffLine<'_>) {
    output.push(prefix);
    output.push_str(line.text);
    output.push('\n');
    if !line.has_newline {
        output.push_str("\\ No newline at end of file\n");
    }
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
        "unknown related_artifacts reference `{trimmed}`; expected ORB-NNNNN, L-NNNN, FYYYY-MM-NNN, or ADR-NNNN"
    ))
}

fn is_task_ref(value: &str) -> bool {
    value.len() == 9
        && value.starts_with("ORB-")
        && value[4..].chars().all(|ch| ch.is_ascii_digit())
}

fn is_learning_ref(value: &str) -> bool {
    let Some(ordinal) = value.strip_prefix("L-") else {
        return false;
    };
    ordinal.len() >= 4 && ordinal.chars().all(|ch| ch.is_ascii_digit())
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

    fn reset_git_check_ignore_invocations() {
        GIT_CHECK_IGNORE_INVOCATIONS.with(|calls| calls.set(0));
    }

    fn git_check_ignore_invocations() -> usize {
        GIT_CHECK_IGNORE_INVOCATIONS.with(std::cell::Cell::get)
    }

    fn yaml_string<'a>(mapping: &'a serde_yaml::Mapping, key: &str) -> Option<&'a str> {
        mapping
            .get(serde_yaml::Value::String(key.to_string()))
            .and_then(serde_yaml::Value::as_str)
    }

    fn apply_patch(root: &Path, diff: &str, dry_run: bool) {
        let mut command = Command::new("patch");
        command.arg("-p0").current_dir(root).stdin(Stdio::piped());
        if dry_run {
            command.arg("--dry-run");
        }
        let mut child = command.spawn().expect("spawn patch");
        child
            .stdin
            .as_mut()
            .expect("patch stdin")
            .write_all(diff.as_bytes())
            .expect("write patch");
        let output = child.wait_with_output().expect("patch output");
        assert!(
            output.status.success(),
            "patch failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn strict_frontmatter_accepts_locked_schema() {
        let parsed = parse_frontmatter(
            "---\ntype: design\nsummary: Hook rewrite design\ntags: [hook, audit]\nrelated_artifacts: [ORB-00160, ADR-0168, L-0003, F2026-05-001]\n---\n# Body\n",
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
    fn walker_batches_git_ignore_once_per_walk() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("docs/nested")).expect("docs dir");
        fs::write(
            root.join("docs/one.md"),
            "---\ntype: context\nsummary: One doc\n---\nbody\n",
        )
        .expect("write one");
        fs::write(
            root.join("docs/nested/two.md"),
            "---\ntype: context\nsummary: Two doc\n---\nbody\n",
        )
        .expect("write two");

        reset_git_check_ignore_invocations();
        let records = walk_docs_roots(root, &["docs/".to_string()]).expect("walk docs");

        assert_eq!(git_check_ignore_invocations(), 1);
        assert_eq!(records.len(), 2);
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
    fn task_context_docs_roots_skip_explicit_empty_or_unset_roots() {
        assert_eq!(
            parse_task_context_docs_roots_from_config_toml("[docs]\n").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            parse_task_context_docs_roots_from_config_toml("[docs]\nroots = []\n").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            parse_task_context_docs_roots_from_config_toml("").unwrap(),
            vec!["docs/"]
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

    #[test]
    fn migration_diff_applies_to_original_content() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let relative = Path::new("docs/design/sample/1_overview.md");
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("create docs");
        let raw = "---\ntitle: Example\nowner: codex\n---\n\n# Example\n";
        fs::write(&path, raw).expect("write original");

        let updated = migrate_doc_content(relative, &path, raw)
            .expect("migrate")
            .expect("changed");
        let diff = migration_diff("docs/design/sample/1_overview.md", raw, &updated);

        apply_patch(root, &diff, true);
        apply_patch(root, &diff, false);
        assert_eq!(fs::read_to_string(&path).expect("read patched"), updated);
    }

    #[test]
    fn migrate_preserves_multiline_frontmatter_values() {
        let raw = "---\ntitle: Example\ndescription: |\n  First line\n  Second: line\nowner: codex\n---\n\n# Example\n";
        let updated = migrate_doc_content(
            Path::new("docs/design/sample/1_overview.md"),
            Path::new("docs/design/sample/1_overview.md"),
            raw,
        )
        .expect("migrate")
        .expect("changed");
        let block = split_frontmatter(&updated)
            .expect("split")
            .expect("frontmatter");
        let yaml = serde_yaml::from_str::<serde_yaml::Value>(block.raw).expect("yaml");
        let mapping = yaml.as_mapping().expect("mapping");

        assert_eq!(
            yaml_string(mapping, "description"),
            Some("First line\nSecond: line\n")
        );
        assert_eq!(yaml_string(mapping, "type"), Some("design"));
        assert_eq!(yaml_string(mapping, "summary"), Some("Example"));
        parse_doc_frontmatter_strict(Path::new("doc.md"), &updated).expect("valid locked schema");
    }

    #[test]
    fn migrate_preserves_quoted_colon_value() {
        let raw = "---\ntitle: \"Foo: bar\"\nowner: codex\n---\n\n# Example\n";
        let updated = migrate_doc_content(
            Path::new("docs/design/sample/1_overview.md"),
            Path::new("docs/design/sample/1_overview.md"),
            raw,
        )
        .expect("migrate")
        .expect("changed");
        let block = split_frontmatter(&updated)
            .expect("split")
            .expect("frontmatter");
        let yaml = serde_yaml::from_str::<serde_yaml::Value>(block.raw).expect("yaml");
        let mapping = yaml.as_mapping().expect("mapping");

        assert_eq!(yaml_string(mapping, "title"), Some("Foo: bar"));
        assert_eq!(yaml_string(mapping, "type"), Some("design"));
        assert_eq!(yaml_string(mapping, "summary"), Some("Example"));
    }
}
