use std::path::Path;

use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::context::RuntimeHost;

use super::git::{git_output, git_success};
use super::input::{canonicalize_existing_dir, input_string_field, required_batch_id};

pub(super) fn push_batch_changes<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = required_batch_id(input, "push_batch_changes")?;
    let workspace_path = match input_string_field(input, "workspace_path") {
        Some(ws) => canonicalize_existing_dir(&ws, "workspace_path")?,
        None => {
            let repo_root_str = host.repo_root()?;
            let repo_root = Path::new(&repo_root_str);
            super::parallel::resolve_shared_worktree_path(repo_root, batch_id)?
        }
    };

    let branch = input_string_field(input, "branch")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])
                .unwrap_or_else(|_| "HEAD".to_string())
                .trim()
                .to_string()
        });

    if branch == "HEAD" {
        return Err(OrbitError::Execution(
            "push_batch_changes: workspace is in detached HEAD state".to_string(),
        ));
    }

    git_success(&workspace_path, &["push", "-u", "origin", &branch])?;
    Ok(json!({}))
}
