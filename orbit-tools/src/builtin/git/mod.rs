pub mod commit;
pub mod push;
pub mod stage_paths;

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use orbit_types::OrbitError;
use serde_json::Value;

use crate::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(stage_paths::GitStagePathsTool);
    registry.register(commit::GitCommitTool);
    registry.register(push::GitPushTool);
}

fn require_repo_root(input: &Value) -> Result<PathBuf, OrbitError> {
    let raw = input
        .get("repo_root")
        .and_then(Value::as_str)
        .ok_or_else(|| OrbitError::InvalidInput("missing `repo_root`".to_string()))?;

    let path = PathBuf::from(raw);
    if !path.exists() {
        return Err(OrbitError::InvalidInput(format!(
            "repo_root does not exist: {raw}"
        )));
    }
    if !path.is_dir() {
        return Err(OrbitError::InvalidInput(format!(
            "repo_root is not a directory: {raw}"
        )));
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| OrbitError::InvalidInput(format!("failed to canonicalize repo_root: {e}")))?;
    if !canonical.join(".git").exists() {
        return Err(OrbitError::InvalidInput(format!(
            "repo_root is not a git repository: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn require_relative_file_paths(input: &Value, repo_root: &Path) -> Result<Vec<String>, OrbitError> {
    let files = input
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| OrbitError::InvalidInput("missing `files`".to_string()))?;
    if files.is_empty() {
        return Err(OrbitError::InvalidInput(
            "`files` must contain at least one path".to_string(),
        ));
    }

    let mut seen = HashSet::new();
    let mut relative_paths = Vec::new();
    for value in files {
        let raw = value
            .as_str()
            .ok_or_else(|| OrbitError::InvalidInput("`files` entries must be strings".to_string()))?
            .trim();
        if raw.is_empty() {
            return Err(OrbitError::InvalidInput(
                "`files` must not contain empty paths".to_string(),
            ));
        }

        let candidate = Path::new(raw);
        let absolute = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            repo_root.join(candidate)
        };

        if !absolute.exists() {
            return Err(OrbitError::InvalidInput(format!(
                "file does not exist: {}",
                absolute.display()
            )));
        }
        let canonical = absolute.canonicalize().map_err(|e| {
            OrbitError::InvalidInput(format!(
                "failed to canonicalize file '{}': {e}",
                absolute.display()
            ))
        })?;
        if !canonical.starts_with(repo_root) {
            return Err(OrbitError::InvalidInput(format!(
                "file is outside repo_root: {}",
                canonical.display()
            )));
        }
        if !canonical.is_file() {
            return Err(OrbitError::InvalidInput(format!(
                "path is not a file: {}",
                canonical.display()
            )));
        }

        let relative = canonical.strip_prefix(repo_root).map_err(|e| {
            OrbitError::InvalidInput(format!("failed to relativize file path: {e}"))
        })?;
        if matches!(
            relative.components().next(),
            Some(Component::Normal(component)) if component == ".git"
        ) {
            return Err(OrbitError::InvalidInput(
                "paths inside .git are not allowed".to_string(),
            ));
        }

        let normalized = relative.to_string_lossy().replace('\\', "/");
        if seen.insert(normalized.clone()) {
            relative_paths.push(normalized);
        }
    }

    Ok(relative_paths)
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::{ToolContext, ToolRegistry};

    fn init_repo() -> std::path::PathBuf {
        let dir = tempdir().expect("temp dir");
        let repo = dir.keep();
        let status = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .expect("git init");
        assert!(status.success(), "git init must succeed");

        let status = Command::new("git")
            .args(["config", "user.email", "orbit@example.com"])
            .current_dir(&repo)
            .status()
            .expect("git config email");
        assert!(status.success(), "git config email must succeed");

        let status = Command::new("git")
            .args(["config", "user.name", "Orbit Test"])
            .current_dir(&repo)
            .status()
            .expect("git config name");
        assert!(status.success(), "git config name must succeed");
        repo
    }

    #[test]
    fn git_tools_stage_and_commit_requested_files() {
        let repo = init_repo();
        let file = repo.join("note.txt");
        std::fs::write(&file, "hello orbit").expect("write file");

        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let staged = registry
            .execute(
                "git.stage_paths",
                &ToolContext::default(),
                json!({
                    "repo_root": repo.to_string_lossy(),
                    "files": ["note.txt"],
                }),
            )
            .expect("stage paths");
        assert_eq!(staged["staged_files"][0], "note.txt");

        let committed = registry
            .execute(
                "git.commit",
                &ToolContext::default(),
                json!({
                    "repo_root": repo.to_string_lossy(),
                    "message": "feat: add note",
                    "files": ["note.txt"],
                }),
            )
            .expect("commit");
        assert_eq!(committed["message"], "feat: add note");
        assert_eq!(committed["committed_files"][0], "note.txt");

        let log = Command::new("git")
            .args(["log", "-1", "--pretty=%B"])
            .current_dir(&repo)
            .output()
            .expect("git log");
        assert_eq!(
            String::from_utf8_lossy(&log.stdout).trim(),
            "feat: add note"
        );
    }

    #[test]
    fn git_stage_paths_rejects_files_outside_repo() {
        let repo = init_repo();
        let external_dir = tempdir().expect("external temp dir");
        let external_file = external_dir.path().join("outside.txt");
        std::fs::write(&external_file, "not allowed").expect("write external file");

        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let result = registry.execute(
            "git.stage_paths",
            &ToolContext::default(),
            json!({
                "repo_root": repo.to_string_lossy(),
                "files": [external_file.to_string_lossy()],
            }),
        );
        let err = result.expect_err("outside file must be rejected");
        assert!(err.to_string().contains("outside repo_root"));
    }

    #[test]
    fn git_commit_only_includes_requested_paths_when_other_changes_are_staged() {
        let repo = init_repo();
        let intended = repo.join("intended.txt");
        let unrelated = repo.join("unrelated.txt");
        std::fs::write(&intended, "intended").expect("write intended file");
        std::fs::write(&unrelated, "unrelated").expect("write unrelated file");

        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        registry
            .execute(
                "git.stage_paths",
                &ToolContext::default(),
                json!({
                    "repo_root": repo.to_string_lossy(),
                    "files": ["unrelated.txt"],
                }),
            )
            .expect("stage unrelated");

        registry
            .execute(
                "git.stage_paths",
                &ToolContext::default(),
                json!({
                    "repo_root": repo.to_string_lossy(),
                    "files": ["intended.txt"],
                }),
            )
            .expect("stage intended");

        registry
            .execute(
                "git.commit",
                &ToolContext::default(),
                json!({
                    "repo_root": repo.to_string_lossy(),
                    "message": "feat: commit intended only",
                    "files": ["intended.txt"],
                }),
            )
            .expect("commit intended");

        let show = Command::new("git")
            .args(["show", "--name-only", "--pretty=", "HEAD"])
            .current_dir(&repo)
            .output()
            .expect("git show");
        let changed = String::from_utf8_lossy(&show.stdout);
        assert!(changed.contains("intended.txt"));
        assert!(!changed.contains("unrelated.txt"));

        let staged = Command::new("git")
            .args(["diff", "--cached", "--name-only"])
            .current_dir(&repo)
            .output()
            .expect("git diff --cached");
        let staged_files = String::from_utf8_lossy(&staged.stdout);
        assert!(staged_files.contains("unrelated.txt"));
        assert!(!staged_files.contains("intended.txt"));
    }
}
