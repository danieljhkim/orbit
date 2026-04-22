//! Shallow structured-config extractor for YAML / JSON / TOML (T20260422-1540).
//!
//! Emits one `LeafKind::ConfigKey` per **top-level** map entry. Nested keys
//! are deliberately out of scope (see the MVP scope in task T20260422-1540);
//! a non-map root (e.g. a JSON top-level array or a YAML scalar) produces zero
//! leaves without erroring.

use super::FileExtractor;
use super::common::{ExtractedLeaf, ExtractionResult, compute_source_hash};
use super::language::{ConfigFormat, FileKind};

pub struct ConfigExtractor {
    format: ConfigFormat,
}

impl ConfigExtractor {
    pub fn new(format: ConfigFormat) -> Self {
        Self { format }
    }
}

impl FileExtractor for ConfigExtractor {
    fn file_kind(&self) -> FileKind {
        FileKind::Config(self.format)
    }

    fn extract(&self, source: &str) -> ExtractionResult {
        let keys = match self.format {
            ConfigFormat::Yaml => extract_yaml_top_level(source),
            ConfigFormat::Json => extract_json_top_level(source),
            ConfigFormat::Toml => extract_toml_top_level(source),
        };
        let mut leaves = Vec::with_capacity(keys.len());
        for (idx, key) in keys.into_iter().enumerate() {
            let hash = compute_source_hash(&key);
            leaves.push(ExtractedLeaf {
                qualified_name: key.clone(),
                name: key,
                kind: "config_key".to_string(),
                start_line: idx + 1,
                end_line: idx + 1,
                source: String::new(),
                source_hash: hash,
                parent_qualified_name: None,
                children_qualified_names: Vec::new(),
                depth: None,
            });
        }
        ExtractionResult { leaves }
    }
}

fn extract_yaml_top_level(source: &str) -> Vec<String> {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(source) else {
        return Vec::new();
    };
    match value {
        serde_yaml::Value::Mapping(map) => map
            .into_iter()
            .filter_map(|(k, _)| yaml_key_as_string(k))
            .collect(),
        _ => Vec::new(),
    }
}

fn yaml_key_as_string(key: serde_yaml::Value) -> Option<String> {
    match key {
        serde_yaml::Value::String(s) => Some(s),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn extract_json_top_level(source: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(source) else {
        return Vec::new();
    };
    match value {
        serde_json::Value::Object(map) => map.into_iter().map(|(k, _)| k).collect(),
        _ => Vec::new(),
    }
}

fn extract_toml_top_level(source: &str) -> Vec<String> {
    let Ok(value) = source.parse::<toml::Value>() else {
        return Vec::new();
    };
    match value {
        toml::Value::Table(map) => map.into_iter().map(|(k, _)| k).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Set-based comparison: per-format map order is an implementation
    /// detail of each parser and not part of the extractor's contract.
    fn sorted_names(result: &ExtractionResult) -> Vec<String> {
        let mut names: Vec<String> = result.leaves.iter().map(|l| l.name.clone()).collect();
        names.sort();
        names
    }

    #[test]
    fn extracts_top_level_yaml_keys_only() {
        let src = "name: orbit\n\
                   version: 0.1.0\n\
                   nested:\n  child: ignored\n\
                   owners:\n  - claude\n  - codex\n";
        let out = ConfigExtractor::new(ConfigFormat::Yaml).extract(src);
        assert_eq!(
            sorted_names(&out),
            vec!["name", "nested", "owners", "version"]
        );
        assert!(out.leaves.iter().all(|l| l.kind == "config_key"));
        assert!(out.leaves.iter().all(|l| l.depth.is_none()));
    }

    #[test]
    fn extracts_top_level_json_keys_only() {
        let src = r#"{"a": 1, "b": {"c": 2}, "d": [1,2]}"#;
        let out = ConfigExtractor::new(ConfigFormat::Json).extract(src);
        assert_eq!(sorted_names(&out), vec!["a", "b", "d"]);
    }

    #[test]
    fn extracts_top_level_toml_keys_only() {
        let src = "name = \"orbit\"\n\
                   version = \"0.1.0\"\n\
                   [package]\n\
                   edition = \"2021\"\n";
        let out = ConfigExtractor::new(ConfigFormat::Toml).extract(src);
        assert_eq!(sorted_names(&out), vec!["name", "package", "version"]);
    }

    #[test]
    fn yaml_non_map_root_produces_no_leaves() {
        let src = "- one\n- two\n- three\n";
        let out = ConfigExtractor::new(ConfigFormat::Yaml).extract(src);
        assert!(out.leaves.is_empty());
    }

    #[test]
    fn json_non_object_root_produces_no_leaves() {
        let src = "[1, 2, 3]";
        let out = ConfigExtractor::new(ConfigFormat::Json).extract(src);
        assert!(out.leaves.is_empty());
    }

    #[test]
    fn malformed_input_produces_no_leaves() {
        let out = ConfigExtractor::new(ConfigFormat::Json).extract("{not valid");
        assert!(out.leaves.is_empty());
        let out = ConfigExtractor::new(ConfigFormat::Yaml).extract("a: b:\n  - [");
        assert!(out.leaves.is_empty());
        let out = ConfigExtractor::new(ConfigFormat::Toml).extract("[unclosed");
        assert!(out.leaves.is_empty());
    }
}
