use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::DEFAULT_ORBITIGNORE_PATTERNS;
use crate::error::KnowledgeError;
use crate::pipeline::context::PipelineContext;

const ORBITIGNORE_FILE_NAME: &str = ".orbitignore";

const SKIP_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "ico", "woff", "woff2", "ttf", "eot", "exe", "dll", "so", "dylib",
    "pdf", "zip", "tar", "gz", "lock",
];

/// Internal .orbitignore matcher used by the scan pipeline.
/// pub(crate) so that sibling tests/scan.rs can exercise it directly (per
/// docs/design-patterns/test_layout.md — tests only see public seams).
pub(crate) struct OrbitIgnoreMatcher {
    gitignore: Gitignore,
}

impl OrbitIgnoreMatcher {
    pub(crate) fn load(repo_path: &Path) -> Result<Self, KnowledgeError> {
        let mut builder = GitignoreBuilder::new(repo_path);
        add_default_orbitignore_patterns(&mut builder)?;

        let discovery_matcher = OrbitIgnoreMatcher {
            gitignore: build_default_orbitignore(repo_path)?,
        };
        let mut orbitignore_files = Vec::new();
        collect_orbitignore_files(
            repo_path,
            repo_path,
            &discovery_matcher,
            &mut orbitignore_files,
        )?;
        orbitignore_files.sort_by(|left, right| {
            let left_rel = left.strip_prefix(repo_path).unwrap_or(left.as_path());
            let right_rel = right.strip_prefix(repo_path).unwrap_or(right.as_path());
            left_rel
                .components()
                .count()
                .cmp(&right_rel.components().count())
                .then_with(|| left_rel.cmp(right_rel))
        });

        for orbitignore in orbitignore_files {
            if let Some(error) = builder.add(&orbitignore) {
                return Err(KnowledgeError::invalid_data(format!(
                    "load {}: {error}",
                    orbitignore.display()
                )));
            }
        }

        let gitignore = builder.build().map_err(|error| {
            KnowledgeError::invalid_data(format!("build .orbitignore matcher: {error}"))
        })?;
        Ok(Self { gitignore })
    }

    pub(crate) fn is_ignored(&self, rel_path: &Path, is_dir: bool) -> bool {
        self.gitignore
            .matched_path_or_any_parents(rel_path, is_dir)
            .is_ignore()
    }
}

fn build_default_orbitignore(repo_path: &Path) -> Result<Gitignore, KnowledgeError> {
    let mut builder = GitignoreBuilder::new(repo_path);
    add_default_orbitignore_patterns(&mut builder)?;
    builder.build().map_err(|error| {
        KnowledgeError::invalid_data(format!("build default .orbitignore matcher: {error}"))
    })
}

fn add_default_orbitignore_patterns(builder: &mut GitignoreBuilder) -> Result<(), KnowledgeError> {
    for pattern in DEFAULT_ORBITIGNORE_PATTERNS {
        builder.add_line(None, pattern).map_err(|error| {
            KnowledgeError::invalid_data(format!(
                "invalid default .orbitignore pattern `{pattern}`: {error}"
            ))
        })?;
    }

    Ok(())
}

/// Scan the repo, populating `ctx.file_paths` with relative paths.
pub fn scan_repo(ctx: &mut PipelineContext) -> Result<(), KnowledgeError> {
    let orbitignore = OrbitIgnoreMatcher::load(&ctx.repo_path)?;
    let mut paths = Vec::new();
    walk_dir(&ctx.repo_path, &ctx.repo_path, &orbitignore, &mut paths)?;
    paths.sort();

    // Filter via git check-ignore
    let ignored = git_ignored_paths(&ctx.repo_path, &paths);
    ctx.file_paths = paths
        .into_iter()
        .filter(|p| !ignored.contains(p))
        .filter(|p| !orbitignore.is_ignored(p, false))
        .collect();

    Ok(())
}

fn walk_dir(
    root: &Path,
    dir: &Path,
    orbitignore: &OrbitIgnoreMatcher,
    out: &mut Vec<PathBuf>,
) -> Result<(), KnowledgeError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| KnowledgeError::io(format!("scan {}: {e}", dir.display())))?;

    for entry in entries {
        let entry =
            entry.map_err(|e| KnowledgeError::io(format!("scan {}: {e}", dir.display())))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        let file_type = entry
            .file_type()
            .map_err(|e| KnowledgeError::io(format!("metadata {}: {e}", path.display())))?;

        if file_type.is_dir() {
            // Skip hidden directories
            if name.starts_with('.') {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(root)
                && orbitignore.is_ignored(rel, true)
            {
                continue;
            }
            walk_dir(root, &path, orbitignore, out)?;
        } else if file_type.is_file() {
            if name.as_ref() == ORBITIGNORE_FILE_NAME {
                continue;
            }
            // Skip hidden files
            if name.starts_with('.') {
                continue;
            }
            // Skip binary/lock extensions
            if let Some(ext) = path.extension().and_then(|e| e.to_str())
                && SKIP_EXTENSIONS.contains(&ext)
            {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(root) {
                if orbitignore.is_ignored(rel, false) {
                    continue;
                }
                out.push(rel.to_path_buf());
            }
        }
    }
    Ok(())
}

fn collect_orbitignore_files(
    root: &Path,
    dir: &Path,
    discovery_matcher: &OrbitIgnoreMatcher,
    out: &mut Vec<PathBuf>,
) -> Result<(), KnowledgeError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| KnowledgeError::io(format!("scan {}: {e}", dir.display())))?;

    for entry in entries {
        let entry =
            entry.map_err(|e| KnowledgeError::io(format!("scan {}: {e}", dir.display())))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        let file_type = entry
            .file_type()
            .map_err(|e| KnowledgeError::io(format!("metadata {}: {e}", path.display())))?;

        if file_type.is_dir() {
            if name.starts_with('.') {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(root)
                && discovery_matcher.is_ignored(rel, true)
            {
                continue;
            }
            collect_orbitignore_files(root, &path, discovery_matcher, out)?;
        } else if file_type.is_file() && name.as_ref() == ORBITIGNORE_FILE_NAME {
            let relative = path
                .strip_prefix(root)
                .map_err(|e| KnowledgeError::io(format!("strip prefix: {e}")))?;
            out.push(root.join(relative));
        }
    }

    Ok(())
}

/// Use `git check-ignore --stdin` to filter git-ignored paths.
fn git_ignored_paths(repo_path: &Path, paths: &[PathBuf]) -> HashSet<PathBuf> {
    let mut ignored = HashSet::new();
    if paths.is_empty() {
        return ignored;
    }

    let stdin_data: String = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");

    let output = Command::new("git")
        .args(["check-ignore", "--stdin"])
        .current_dir(repo_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(stdin_data.as_bytes());
            }
            child.wait_with_output()
        });

    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                ignored.insert(PathBuf::from(trimmed));
            }
        }
    }

    ignored
}
