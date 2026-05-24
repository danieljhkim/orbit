#![allow(missing_docs)]

use std::path::Path;

use crate::Extractor;
use crate::languages::ConfigExtractor;

fn extract_yaml(source: &str) -> crate::ExtractedFile {
    ConfigExtractor.extract(Path::new("app.yaml"), source.as_bytes())
}
fn extract_toml(source: &str) -> crate::ExtractedFile {
    ConfigExtractor.extract(Path::new("Cargo.toml"), source.as_bytes())
}
fn extract_json(source: &str) -> crate::ExtractedFile {
    ConfigExtractor.extract(Path::new("settings.json"), source.as_bytes())
}

#[test]
fn config_yaml_toml_json_each_populate_configs_with_correct_kind() {
    let yml = "server:\n  port: 8080\n  host: localhost\nfeatures:\n  - alpha\n";
    let file = extract_yaml(yml);
    assert!(!file.configs.is_empty());
    assert!(file.configs.iter().any(|c| c.kind == "yaml" && c.key.contains("server")));
    assert!(file.relations.is_empty() && file.refs.is_empty()); // config never emits these

    let toml_src = "[package]\nname = \"test\"\n\n[dependencies]\nfoo = \"1\"\n";
    let file = extract_toml(toml_src);
    assert!(file.configs.iter().any(|c| c.kind == "toml" && c.key.contains("package")));
    assert!(file.refs.is_empty() && file.relations.is_empty());

    let json = r#"{"db": {"url": "postgres://"}, "debug": true}"#;
    let file = extract_json(json);
    assert!(file.configs.iter().any(|c| c.kind == "json" && c.key.contains("db")));
    assert!(file.relations.is_empty());
}

#[test]
fn config_env_parses_keys() {
    let env = "DB_HOST=localhost\nPORT=3000\n# comment\n";
    let file = ConfigExtractor.extract(Path::new(".env"), env.as_bytes());
    assert!(file.configs.iter().any(|c| c.kind == "env" && c.key == "DB_HOST"));
    assert!(file.configs.iter().any(|c| c.key == "PORT"));
}
