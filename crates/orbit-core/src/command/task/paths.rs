use chrono::Utc;
use orbit_types::{OrbitError, TaskHistoryEntry};
use std::path::{Path, PathBuf};

pub(super) fn normalize_workspace_path(
    repo_root: &Path,
    workspace: Option<&str>,
) -> Result<Option<String>, OrbitError> {
    let Some(workspace) = workspace.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let canonical_repo_root = repo_root.canonicalize().map_err(|error| {
        OrbitError::InvalidInput(format!(
            "failed to resolve repository root '{}': {error}",
            repo_root.display()
        ))
    })?;
    let candidate = if Path::new(workspace).is_absolute() {
        PathBuf::from(workspace)
    } else {
        canonical_repo_root.join(workspace)
    };
    let canonical_workspace = candidate.canonicalize().map_err(|error| {
        OrbitError::InvalidInput(format!(
            "workspace_path '{}' must reference an existing directory inside the repository: {error}",
            candidate.display()
        ))
    })?;
    if !canonical_workspace.is_dir() {
        return Err(OrbitError::InvalidInput(format!(
            "workspace_path '{}' must reference a directory inside the repository",
            canonical_workspace.display()
        )));
    }

    if !canonical_workspace.starts_with(&canonical_repo_root) {
        return Err(OrbitError::InvalidInput(format!(
            "workspace_path '{}' must stay within repository '{}'",
            canonical_workspace.display(),
            canonical_repo_root.display()
        )));
    }

    Ok(Some(canonical_workspace.to_string_lossy().into_owned()))
}

pub(super) fn context_workspace_root(repo_root: &Path, workspace_path: Option<&str>) -> PathBuf {
    workspace_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.to_path_buf())
}

pub(super) fn context_files_pruned_history_entry(
    actor: &str,
    dropped: &[String],
) -> TaskHistoryEntry {
    TaskHistoryEntry {
        at: Utc::now(),
        by: actor.to_string(),
        event: "context_files_pruned".to_string(),
        note: Some(format!(
            "dropped: {} (not found in workspace)",
            dropped.join(", ")
        )),
        from_status: None,
        to_status: None,
    }
}

pub(super) fn extract_task_path_mentions(text: &str) -> Vec<String> {
    let mut paths = std::collections::BTreeSet::new();
    for raw in text.split_whitespace() {
        let trimmed = raw.trim_matches(|ch: char| {
            matches!(
                ch,
                '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ':' | ';'
            )
        });
        let trimmed = trimmed.trim_end_matches(&['.', '!', '?'][..]);
        if let Some(path) = normalize_path_token(trimmed) {
            paths.insert(path);
        }
    }
    paths.into_iter().collect()
}

pub(super) fn normalize_path_token(token: &str) -> Option<String> {
    if token.is_empty() || token.contains("://") {
        return None;
    }

    let token = token
        .strip_prefix("file:")
        .or_else(|| token.strip_prefix("dir:"))
        .or_else(|| token.strip_prefix("symbol:"))
        .unwrap_or(token);
    let token = token.split_once('#').map(|(path, _)| path).unwrap_or(token);
    let token = token.trim_matches('`').trim_end_matches('/');
    if token.is_empty() {
        return None;
    }

    let standalone_files = [
        "Cargo.toml",
        "Cargo.lock",
        "Makefile",
        "README.md",
        "AGENTS.md",
        "CLAUDE.md",
    ];
    let has_known_prefix = [
        "./",
        "../",
        "crates/",
        "src/",
        "tests/",
        "scripts/",
        "docs/",
        "examples/",
        ".orbit/",
    ]
    .iter()
    .any(|prefix| token.starts_with(prefix));
    let last_segment_looks_like_file = token
        .rsplit('/')
        .next()
        .is_some_and(|segment| segment.contains('.'));

    if has_known_prefix
        || standalone_files.contains(&token)
        || (token.contains('/') && last_segment_looks_like_file)
    {
        return Some(token.to_string());
    }

    None
}

pub(super) fn task_path_exists(workspace_root: &Path, raw_path: &str) -> bool {
    let candidate = Path::new(raw_path.trim());
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    };
    resolved.exists()
}
