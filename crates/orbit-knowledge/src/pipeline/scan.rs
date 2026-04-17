use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::KnowledgeError;
use crate::pipeline::context::PipelineContext;

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "__pycache__",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    ".egg-info",
];

const SKIP_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "ico", "woff", "woff2", "ttf", "eot", "exe", "dll", "so", "dylib",
    "pdf", "zip", "tar", "gz", "lock",
];

/// Scan the repo, populating `ctx.file_paths` with relative paths.
pub fn scan_repo(ctx: &mut PipelineContext) -> Result<(), KnowledgeError> {
    let mut paths = Vec::new();
    walk_dir(&ctx.repo_path, &ctx.repo_path, &mut paths)?;
    paths.sort();

    // Filter via git check-ignore
    let ignored = git_ignored_paths(&ctx.repo_path, &paths);
    ctx.file_paths = paths.into_iter().filter(|p| !ignored.contains(p)).collect();

    Ok(())
}

fn walk_dir(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), KnowledgeError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| KnowledgeError::io(format!("scan {}: {e}", dir.display())))?;

    for entry in entries {
        let entry =
            entry.map_err(|e| KnowledgeError::io(format!("scan {}: {e}", dir.display())))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        if path.is_dir() {
            // Skip hidden directories
            if name.starts_with('.') {
                continue;
            }
            // Skip known non-source directories
            if SKIP_DIRS.iter().any(|d| name.as_ref() == *d) {
                continue;
            }
            walk_dir(root, &path, out)?;
        } else if path.is_file() {
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
                out.push(rel.to_path_buf());
            }
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
