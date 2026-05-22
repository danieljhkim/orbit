use std::path::Path;

use serde_json::json;

use orbit_common::types::OrbitError;

use super::config::parse_docs_roots_from_config_toml;
use super::path_util::path_to_slash_string;
use super::types::DocAddOutcome;

pub(super) fn add_docs_root(
    repo_root: &Path,
    config_path: &Path,
    path: &str,
) -> Result<DocAddOutcome, OrbitError> {
    let normalized = normalize_docs_root_arg(repo_root, path)?;
    let raw = if config_path.exists() {
        std::fs::read_to_string(config_path)
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
        std::fs::create_dir_all(parent)
            .map_err(|error| OrbitError::Io(format!("create {}: {error}", parent.display())))?;
    }
    std::fs::write(config_path, rendered)
        .map_err(|error| OrbitError::Io(format!("write {}: {error}", config_path.display())))
}
