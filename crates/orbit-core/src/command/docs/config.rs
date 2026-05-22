use std::path::Path;

use serde::Deserialize;
use serde_json::json;

use orbit_common::types::OrbitError;

const DEFAULT_DOC_ROOT: &str = "docs/";

#[derive(Debug, Deserialize)]
struct DocsConfigFile {
    docs: Option<DocsConfigSection>,
}

#[derive(Debug, Deserialize)]
struct DocsConfigSection {
    roots: Option<Vec<String>>,
    search: Option<DocsSearchConfigSection>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DocsSearchConfig {
    pub semantic_weight: f32,
}

impl Default for DocsSearchConfig {
    fn default() -> Self {
        Self {
            semantic_weight: 0.5,
        }
    }
}

#[derive(Debug, Deserialize)]
struct DocsSearchConfigSection {
    semantic_weight: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct AdrConfigFile {
    adr: Option<AdrConfigSection>,
}

#[derive(Debug, Deserialize)]
struct AdrConfigSection {
    search: Option<AdrSearchConfigSection>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdrSearchConfig {
    pub semantic_weight: f32,
}

impl Default for AdrSearchConfig {
    fn default() -> Self {
        Self {
            semantic_weight: 0.5,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AdrSearchConfigSection {
    semantic_weight: Option<f32>,
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

pub fn parse_docs_search_config_from_config_toml(
    raw: &str,
) -> Result<DocsSearchConfig, OrbitError> {
    if raw.trim().is_empty() {
        return Ok(DocsSearchConfig::default());
    }
    let parsed = toml::from_str::<DocsConfigFile>(raw).map_err(|error| {
        OrbitError::InvalidInput(format!("invalid docs config in config.toml: {error}"))
    })?;
    let semantic_weight = parsed
        .docs
        .and_then(|section| section.search)
        .and_then(|section| section.semantic_weight)
        .unwrap_or_else(|| DocsSearchConfig::default().semantic_weight)
        .clamp(0.0, 1.0);
    Ok(DocsSearchConfig { semantic_weight })
}

pub fn parse_adr_search_config_from_config_toml(raw: &str) -> Result<AdrSearchConfig, OrbitError> {
    if raw.trim().is_empty() {
        return Ok(AdrSearchConfig::default());
    }
    let parsed = toml::from_str::<AdrConfigFile>(raw).map_err(|error| {
        OrbitError::InvalidInput(format!("invalid ADR config in config.toml: {error}"))
    })?;
    let semantic_weight = parsed
        .adr
        .and_then(|section| section.search)
        .and_then(|section| section.semantic_weight)
        .unwrap_or_else(|| AdrSearchConfig::default().semantic_weight)
        .clamp(0.0, 1.0);
    Ok(AdrSearchConfig { semantic_weight })
}

pub(super) fn read_docs_roots_from_config_path(path: &Path) -> Result<Vec<String>, OrbitError> {
    if !path.exists() {
        return Ok(default_doc_roots());
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
    parse_docs_roots_from_config_toml(&raw)
}

pub(super) fn read_docs_search_config_from_config_path(
    path: &Path,
) -> Result<DocsSearchConfig, OrbitError> {
    if !path.exists() {
        return Ok(DocsSearchConfig::default());
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
    parse_docs_search_config_from_config_toml(&raw)
}

pub(super) fn read_adr_search_config_from_config_path(
    path: &Path,
) -> Result<AdrSearchConfig, OrbitError> {
    if !path.exists() {
        return Ok(AdrSearchConfig::default());
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
    parse_adr_search_config_from_config_toml(&raw)
}

pub(super) fn read_task_context_docs_roots_from_config_path(
    path: &Path,
) -> Result<Vec<String>, OrbitError> {
    if !path.exists() {
        return Ok(default_doc_roots());
    }
    let raw = std::fs::read_to_string(path)
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
