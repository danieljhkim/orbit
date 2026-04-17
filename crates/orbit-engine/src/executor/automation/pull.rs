use std::path::Path;

use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::context::RuntimeHost;

use super::git::git_success;
use super::input::{canonicalize_existing_dir, input_string_field, required_batch_id};

pub(super) fn pull_batch_changes<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = required_batch_id(input, "pull_batch_changes")?;
    let workspace_path = match input_string_field(input, "workspace_path") {
        Some(ws) => canonicalize_existing_dir(&ws, "workspace_path")?,
        None => {
            let repo_root_str = host.repo_root()?;
            let repo_root = Path::new(&repo_root_str);
            super::parallel::resolve_shared_worktree_path(repo_root, batch_id)?
        }
    };

    git_success(&workspace_path, &["pull", "--rebase"])?;
    Ok(json!({}))
}
