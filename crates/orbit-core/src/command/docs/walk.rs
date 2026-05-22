use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use orbit_common::types::OrbitError;

use super::frontmatter::parse_doc_tolerant;
use super::path_util::{path_to_slash_string, repo_relative_path};
use super::types::DocRecord;

#[cfg(test)]
thread_local! {
    static GIT_CHECK_IGNORE_INVOCATIONS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn record_git_check_ignore_invocation() {
    GIT_CHECK_IGNORE_INVOCATIONS.with(|calls| calls.set(calls.get() + 1));
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

pub(super) fn expand_root(repo_root: &Path, root: &str) -> Result<Vec<PathBuf>, OrbitError> {
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
        .filter_map(super::path_util::component_str)
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

pub(crate) fn path_is_or_contains_dot_orbit(repo_root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(repo_root).unwrap_or(path);
    relative.components().any(
        |component| matches!(component, std::path::Component::Normal(value) if value == ".orbit"),
    )
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
