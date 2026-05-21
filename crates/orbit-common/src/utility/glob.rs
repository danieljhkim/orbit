//! Glob-matching helpers shared between policy evaluation and learning scope.
//!
//! Originally lived as private helpers inside
//! [`crate::types::policy_def`]; promoted here so `orbit-store`'s
//! `learning_store` can reuse the same matcher without taking a dependency
//! on `orbit-policy` (which the architecture diagram forbids). The original
//! call sites in `policy_def` now route through these public functions, so
//! existing behavior is unchanged.
//!
//! # Semantics
//! The grammar supports the three glob operators agents already see in
//! `denyRead`/`denyModify` rules and `fsProfile` patterns:
//! - `**` — any number of path segments (including zero).
//! - `*` — any sequence of non-separator characters.
//! - `?` — any single non-separator character.
//!
//! All inputs are normalized to forward-slash separators and stripped of
//! leading `./`. Paths that escape the workspace (`..`, `~`, absolute) are
//! rejected as `OrbitError::InvalidInput`.

use std::path::{Component, Path};

use regex::Regex;

use crate::types::OrbitError;

/// Normalize a workspace-relative path for glob matching.
///
/// Trims whitespace, replaces backslashes with forward slashes, strips a
/// leading `./`, and rejects paths that try to escape the workspace
/// (`..`, `~`, absolute). Returns an empty string for the workspace root
/// (`.`).
pub fn normalize_glob_path(path: &str) -> Result<String, OrbitError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(
            "filesystem path must not be empty".to_string(),
        ));
    }

    let mut normalized = trimmed.replace('\\', "/");
    while let Some(stripped) = normalized.strip_prefix("./") {
        normalized = stripped.to_string();
    }
    if normalized == "." {
        normalized.clear();
    }
    if normalized == "~"
        || normalized.starts_with("~/")
        || normalized.starts_with("../")
        || normalized == ".."
    {
        return Err(OrbitError::InvalidInput(format!(
            "filesystem path `{path}` must stay inside the workspace root"
        )));
    }

    let path_ref = Path::new(&normalized);
    if path_ref.is_absolute() {
        return Err(OrbitError::InvalidInput(format!(
            "filesystem path `{path}` must stay inside the workspace root"
        )));
    }

    for component in path_ref.components() {
        match component {
            Component::CurDir | Component::Normal(_) => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(OrbitError::InvalidInput(format!(
                    "filesystem path `{path}` must stay inside the workspace root"
                )));
            }
        }
    }

    Ok(normalized)
}

/// Test whether `rule` (a glob pattern) matches `path` (already normalized
/// via [`normalize_glob_path`]).
pub fn match_glob(rule: &str, path: &str) -> Result<bool, OrbitError> {
    let regex = compile_glob_regex(rule).map_err(|error| {
        OrbitError::InvalidInput(format!("invalid filesystem glob `{rule}`: {error}"))
    })?;
    Ok(regex.is_match(path))
}

/// Compile a glob pattern into an anchored regex.
///
/// Exposed as a building block for callers that want to cache compiled
/// regexes across many path checks (e.g. evaluating a single rule against
/// hundreds of candidate paths in a hot path).
pub fn compile_glob_regex(rule: &str) -> Result<Regex, regex::Error> {
    if rule == "." {
        return Regex::new(r"^$");
    }

    if let Some(prefix) = rule.strip_suffix("/**") {
        if prefix.is_empty() {
            return Regex::new(r"^.*$");
        }
        let escaped = regex::escape(prefix);
        return Regex::new(&format!("^{escaped}(?:/.*)?$"));
    }

    let chars: Vec<char> = rule.chars().collect();
    let mut index = 0usize;
    let mut pattern = String::from("^");
    while index < chars.len() {
        if chars[index] == '*' {
            if index + 2 < chars.len() && chars[index + 1] == '*' && chars[index + 2] == '/' {
                pattern.push_str("(?:.*/)?");
                index += 3;
                continue;
            }
            if index + 1 < chars.len() && chars[index + 1] == '*' {
                pattern.push_str(".*");
                index += 2;
                continue;
            }
            pattern.push_str("[^/]*");
            index += 1;
            continue;
        }

        if chars[index] == '?' {
            pattern.push_str("[^/]");
            index += 1;
            continue;
        }

        pattern.push_str(&regex::escape(&chars[index].to_string()));
        index += 1;
    }
    pattern.push('$');
    Regex::new(&pattern)
}

#[cfg(test)]
mod tests;
