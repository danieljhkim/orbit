//! Config extractor for orbit-graph-extract (ORB-00305).
//!
//! Populates ExtractedFile::configs (never refs/relations).
//! Uses workspace serde_yaml/toml/serde_json for real parsing + line scan fallback for positions.
//! Kinds: yaml|toml|json|env|serde per spec §6.2.

use std::path::Path;

use crate::{ExtractedFile, Extractor, RawConfig};
use super::common::normalize_path;

/// Config (yaml/toml/json/env) key extractor.
pub struct ConfigExtractor;

impl Extractor for ConfigExtractor {
    fn lang(&self) -> &'static str {
        "config"
    }

    fn supports(&self, path: &Path) -> bool {
        matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yaml") | Some("yml") | Some("toml") | Some("json") | Some("env")
        )
    }

    fn extract(&self, path: &Path, bytes: &[u8]) -> ExtractedFile {
        let Ok(source) = std::str::from_utf8(bytes) else {
            return ExtractedFile::default();
        };

        let kind = detect_kind(path);
        let mut configs = Vec::new();

        match kind.as_str() {
            "toml" => {
                if let Ok(table) = source.parse::<toml::Table>() {
                    collect_toml_keys(&table, "", 1, &mut configs, path, &kind);
                } else {
                    scan_keys(source, &mut configs, path, &kind);
                }
            }
            "yaml" | "yml" => {
                if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(source) {
                    collect_yaml_keys(&value, "", 1, &mut configs, path, &kind);
                } else {
                    scan_keys(source, &mut configs, path, &kind);
                }
            }
            "json" => {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(source) {
                    collect_json_keys(&value, "", 1, &mut configs, path, &kind);
                } else {
                    scan_keys(source, &mut configs, path, &kind);
                }
            }
            "env" => {
                scan_keys(source, &mut configs, path, &kind);
            }
            _ => scan_keys(source, &mut configs, path, &kind),
        }

        ExtractedFile {
            configs,
            ..Default::default()
        }
    }
}

fn detect_kind(path: &Path) -> String {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        match ext {
            "yaml" | "yml" => return "yaml".to_string(),
            "toml" => return "toml".to_string(),
            "json" => return "json".to_string(),
            "env" => return "env".to_string(),
            _ => {}
        }
    }
    // handle dotfiles like .env (no extension per Path)
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name == ".env" || name.ends_with(".env") {
            return "env".to_string();
        }
    }
    "serde".to_string()
}

fn scan_keys(source: &str, out: &mut Vec<RawConfig>, path: &Path, kind: &str) {
    let file_path = normalize_path(path);
    for (i, line) in source.lines().enumerate() {
        let t = line.trim_start();
        if let Some(colon) = t.find(':') {
            let key = t[..colon].trim().to_string();
            if !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
                out.push(RawConfig { file_path: file_path.clone(), line: i + 1, key, kind: kind.to_string() });
            }
        } else if let Some(eq) = t.find('=') {
            let key = t[..eq].trim().to_string();
            if !key.is_empty() {
                out.push(RawConfig { file_path: file_path.clone(), line: i + 1, key, kind: kind.to_string() });
            }
        }
    }
}

fn collect_toml_keys(table: &toml::Table, prefix: &str, line: usize, out: &mut Vec<RawConfig>, path: &Path, kind: &str) {
    let file_path = normalize_path(path);
    for (k, v) in table {
        let full = if prefix.is_empty() { k.clone() } else { format!("{}.{}", prefix, k) };
        out.push(RawConfig { file_path: file_path.clone(), line, key: full.clone(), kind: kind.to_string() });
        if let toml::Value::Table(sub) = v {
            collect_toml_keys(sub, &full, line, out, path, kind);
        }
    }
}

fn collect_yaml_keys(value: &serde_yaml::Value, prefix: &str, line: usize, out: &mut Vec<RawConfig>, path: &Path, kind: &str) {
    let file_path = normalize_path(path);
    match value {
        serde_yaml::Value::Mapping(map) => {
            for (k, v) in map {
                if let Some(kstr) = k.as_str() {
                    let full = if prefix.is_empty() { kstr.to_string() } else { format!("{}.{}", prefix, kstr) };
                    out.push(RawConfig { file_path: file_path.clone(), line, key: full.clone(), kind: kind.to_string() });
                    collect_yaml_keys(v, &full, line, out, path, kind);
                }
            }
        }
        _ => {}
    }
}

fn collect_json_keys(value: &serde_json::Value, prefix: &str, line: usize, out: &mut Vec<RawConfig>, path: &Path, kind: &str) {
    let file_path = normalize_path(path);
    if let serde_json::Value::Object(map) = value {
        for (k, v) in map {
            let full = if prefix.is_empty() { k.clone() } else { format!("{}.{}", prefix, k) };
            out.push(RawConfig { file_path: file_path.clone(), line, key: full.clone(), kind: kind.to_string() });
            collect_json_keys(v, &full, line, out, path, kind);
        }
    }
}
