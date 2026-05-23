use std::path::Path;

use orbit_common::types::OrbitError;
use rusqlite::Connection;

use super::queries::workspace_by_id;
use super::util::normalize_path;

pub(super) fn next_workspace_id_candidate(
    conn: &Connection,
    slug: &str,
    path: &Path,
) -> Result<String, OrbitError> {
    for attempt in 0..1000 {
        let candidate = workspace_id_candidate(slug, path, attempt);
        if workspace_by_id(conn, &candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(OrbitError::Store(format!(
        "could not allocate workspace id for slug '{slug}'"
    )))
}

pub(super) fn workspace_id_candidate(slug: &str, path: &Path, attempt: u32) -> String {
    let input = format!("{}:{}:{attempt}", slug, normalize_path(path).display());
    let hash = blake3::hash(input.as_bytes()).to_hex();
    format!("{slug}-{}", &hash[..6])
}

pub(super) fn sanitize_slug(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in raw.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "workspace".to_string()
    } else {
        out
    }
}

pub(super) fn validate_workspace_id(raw: &str) -> Result<String, OrbitError> {
    let trimmed = raw.trim();
    let Some((slug, suffix)) = trimmed.rsplit_once('-') else {
        return Err(OrbitError::InvalidInput(format!(
            "workspace_id '{trimmed}' must use <slug>-<6char> form"
        )));
    };
    if !is_valid_workspace_slug(slug)
        || suffix.len() != 6
        || !suffix
            .as_bytes()
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(OrbitError::InvalidInput(format!(
            "workspace_id '{trimmed}' must use <slug>-<6char> form"
        )));
    }
    Ok(trimmed.to_string())
}

fn is_valid_workspace_slug(slug: &str) -> bool {
    if slug.is_empty() || slug.starts_with('-') || slug.ends_with('-') || slug.contains("--") {
        return false;
    }
    slug.as_bytes()
        .iter()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
}
