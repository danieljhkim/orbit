//! Public `OrbitRuntime` surface for the project-learnings CLI subcommands.
//!
//! Mirrors the helpers used by `orbit.learning.*` MCP tools but lives on the
//! runtime so `orbit-cli` can call them without depending on the host-side
//! dispatch layer. Tool-host and CLI both reach into
//! `runtime.stores().learnings()`, which is the single source of truth.

use std::path::{Path, PathBuf};

use orbit_common::types::{
    EvidenceKind, Learning, LearningComment, LearningStatus, NotFoundKind, OrbitError,
};
use orbit_common::types::{
    LearningVoteSummary, all_agent_families, normalize_agent_family_for_model,
};
use orbit_store::{
    LearningCommentAddParams, LearningCommentDeleteParams, LearningCreateParams,
    LearningSearchParams, LearningSearchResult, LearningUpdateParams, LearningUpvoteParams,
    learning_layout::LearningLayoutMigrationReport,
};

use crate::OrbitRuntime;

impl OrbitRuntime {
    pub fn create_learning(&self, params: LearningCreateParams) -> Result<Learning, OrbitError> {
        self.stores().learnings().add(params)
    }

    pub fn get_learning(&self, id: &str) -> Result<Learning, OrbitError> {
        self.stores()
            .learnings()
            .get(id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Learning, id.to_string()))
    }

    pub fn list_learnings(
        &self,
        status: Option<LearningStatus>,
    ) -> Result<Vec<Learning>, OrbitError> {
        self.stores().learnings().list(status)
    }

    pub fn search_learnings(
        &self,
        params: LearningSearchParams,
    ) -> Result<Vec<LearningSearchResult>, OrbitError> {
        let params = normalize_learning_search_params(&self.paths().repo_root, params)?;
        self.stores().learnings().search(params)
    }

    pub fn upvote_learning(
        &self,
        params: LearningUpvoteParams,
    ) -> Result<LearningVoteSummary, OrbitError> {
        let voter_model = normalize_learning_voter_model(&params.voter_model)?;
        self.stores().learnings().upvote(LearningUpvoteParams {
            voter_model,
            ..params
        })
    }

    pub fn learning_vote_summary(&self, id: &str) -> Result<LearningVoteSummary, OrbitError> {
        self.stores().learnings().vote_summary(id)
    }

    pub fn add_learning_comment(
        &self,
        learning_id: String,
        body: String,
        model: String,
    ) -> Result<LearningComment, OrbitError> {
        let author_model = normalize_learning_agent_model(&model)?;
        self.stores()
            .learnings()
            .add_comment(LearningCommentAddParams {
                learning_id,
                body,
                author_model,
            })
    }

    pub fn list_learning_comments(
        &self,
        learning_id: &str,
        include_deleted: bool,
    ) -> Result<Vec<LearningComment>, OrbitError> {
        self.stores()
            .learnings()
            .list_comments(learning_id, include_deleted)
    }

    pub fn delete_learning_comment(
        &self,
        comment_id: String,
        deleted_by: Option<String>,
    ) -> Result<(), OrbitError> {
        let deleted_by = match deleted_by {
            Some(model) => normalize_learning_agent_model(&model)?,
            None => "unknown".to_string(),
        };
        self.stores()
            .learnings()
            .delete_comment(LearningCommentDeleteParams {
                comment_id,
                deleted_by,
            })
    }

    pub fn update_learning(
        &self,
        id: &str,
        params: LearningUpdateParams,
    ) -> Result<Learning, OrbitError> {
        self.stores().learnings().update(id, params)
    }

    pub fn supersede_learning(&self, old_id: &str, new_id: &str) -> Result<(), OrbitError> {
        if old_id == new_id {
            return Err(OrbitError::InvalidInput(format!(
                "learning '{old_id}' cannot supersede itself"
            )));
        }
        self.stores().learnings().supersede(old_id, new_id)
    }

    pub fn archive_learning(&self, id: &str) -> Result<bool, OrbitError> {
        self.stores().learnings().archive(id)
    }

    pub fn reindex_learnings(&self) -> Result<(), OrbitError> {
        self.stores().learnings().reindex()
    }

    pub fn migrate_learning_layout(&self) -> Result<LearningLayoutMigrationReport, OrbitError> {
        migrate_learning_layout_at(&self.paths().orbit_dir)
    }

    /// Returns the IDs of every active learning that the §7.3 staleness
    /// rules flag as stale. A learning is stale when ALL of:
    /// * every `scope.paths` glob resolves to no extant directory under
    ///   the repo root,
    /// * every evidence task ID is unknown to the workspace task store, AND
    /// * every evidence commit SHA is unknown to git.
    ///
    /// A learning with no scope paths AND no evidence is NOT stale.
    pub fn stale_learning_ids(&self) -> Result<Vec<String>, OrbitError> {
        let active = self.list_learnings(Some(LearningStatus::Active))?;
        let repo_root = self.paths().repo_root.clone();
        Ok(active
            .iter()
            .filter(|l| is_learning_stale(self, l, &repo_root))
            .map(|l| l.id.clone())
            .collect())
    }

    /// Archive every stale active learning per `stale_learning_ids`. Returns
    /// `{ stale, deleted }` as a parallel pair of ID lists.
    pub fn prune_learnings(&self, delete: bool) -> Result<(Vec<String>, Vec<String>), OrbitError> {
        let stale = self.stale_learning_ids()?;
        let mut deleted = Vec::new();
        if delete {
            for id in &stale {
                self.archive_learning(id)?;
                deleted.push(id.clone());
            }
        }
        Ok((stale, deleted))
    }
}

pub fn migrate_learning_layout_at(
    workspace_orbit_dir: &Path,
) -> Result<LearningLayoutMigrationReport, OrbitError> {
    orbit_store::learning_layout::migrate_learning_layout(
        &workspace_orbit_dir.join("learnings"),
        workspace_orbit_dir,
    )
}

fn is_learning_stale(runtime: &OrbitRuntime, learning: &Learning, repo_root: &Path) -> bool {
    if learning.scope.paths.is_empty() && learning.evidence.is_empty() {
        return false;
    }
    let paths_stale = learning.scope.paths.is_empty()
        || learning
            .scope
            .paths
            .iter()
            .all(|glob| !glob_has_extant_prefix(repo_root, glob));

    let mut evidence_stale = true;
    for ev in &learning.evidence {
        let alive = match ev.kind {
            EvidenceKind::Task => runtime
                .stores()
                .tasks()
                .get(&ev.reference)
                .ok()
                .flatten()
                .is_some(),
            EvidenceKind::Commit => commit_sha_known(repo_root, &ev.reference),
            EvidenceKind::External => true,
        };
        if alive {
            evidence_stale = false;
            break;
        }
    }
    if learning.evidence.is_empty() {
        evidence_stale = true;
    }
    paths_stale && evidence_stale
}

fn glob_has_extant_prefix(repo_root: &Path, glob: &str) -> bool {
    let trimmed = glob.trim_start_matches('/');
    let prefix: String = trimmed
        .split('/')
        .take_while(|segment| {
            !segment.contains('*') && !segment.contains('?') && !segment.contains('[')
        })
        .collect::<Vec<_>>()
        .join("/");
    if prefix.is_empty() {
        return repo_root.exists();
    }
    repo_root.join(prefix).exists()
}

fn commit_sha_known(repo_root: &Path, sha: &str) -> bool {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("cat-file")
        .arg("-e")
        .arg(sha)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    matches!(status, Ok(status) if status.success())
}

fn normalize_learning_search_params(
    repo_root: &Path,
    mut params: LearningSearchParams,
) -> Result<LearningSearchParams, OrbitError> {
    if let Some(path) = params.path.as_deref() {
        params.path = Some(normalize_learning_search_path(repo_root, path)?);
    }
    Ok(params)
}

fn normalize_learning_search_path(repo_root: &Path, path: &str) -> Result<String, OrbitError> {
    let trimmed = path.trim();
    let candidate = Path::new(trimmed);
    if !candidate.is_absolute() {
        return Ok(path.to_string());
    }

    let canonical_repo_root = canonicalize_with_missing_tail(repo_root)?;
    let canonical_candidate = canonicalize_with_missing_tail(candidate)?;
    if let Ok(relative) = canonical_candidate.strip_prefix(&canonical_repo_root) {
        return Ok(workspace_relative_path_string(relative));
    }

    if let Some(relative) =
        linked_worktree_relative_path(&canonical_repo_root, candidate, &canonical_candidate)
    {
        return Ok(relative);
    }

    Err(OrbitError::InvalidInput(format!(
        "filesystem path `{path}` must stay inside the workspace root"
    )))
}

fn normalize_learning_voter_model(raw: &str) -> Result<String, OrbitError> {
    normalize_learning_agent_model(raw)
}

fn normalize_learning_agent_model(raw: &str) -> Result<String, OrbitError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(
            "learning action requires a non-empty model".to_string(),
        ));
    }
    if let Some(family) = normalize_agent_family_for_model(None, Some(trimmed))? {
        return Ok(family);
    }
    let family = normalize_agent_family_for_model(Some(trimmed), None)?;
    if let Some(family) = family
        && all_agent_families().contains(&family.as_str())
    {
        return Ok(family);
    }
    Err(OrbitError::InvalidInput(format!(
        "unknown agent model `{trimmed}`; use a canonical family (codex, claude, gemini, grok) or a recognized model name"
    )))
}

fn workspace_relative_path_string(relative: &Path) -> String {
    if relative.as_os_str().is_empty() {
        ".".to_string()
    } else {
        relative.to_string_lossy().replace('\\', "/")
    }
}

fn linked_worktree_relative_path(
    canonical_repo_root: &Path,
    candidate: &Path,
    canonical_candidate: &Path,
) -> Option<String> {
    let checkout_root = git_checkout_root(candidate)?;
    let main_root = crate::paths::find_git_main_worktree_root(&checkout_root)?;
    let canonical_main_root = canonicalize_with_missing_tail(&main_root).ok()?;
    if canonical_main_root != canonical_repo_root {
        return None;
    }

    let canonical_checkout_root = canonicalize_with_missing_tail(&checkout_root).ok()?;
    let relative = canonical_candidate
        .strip_prefix(canonical_checkout_root)
        .ok()?;
    Some(workspace_relative_path_string(relative))
}

fn git_checkout_root(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        let git_path = ancestor.join(".git");
        if git_path.is_dir() || git_path.is_file() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn canonicalize_with_missing_tail(path: &Path) -> Result<PathBuf, OrbitError> {
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|error| OrbitError::Io(format!("failed to canonicalize path: {error}")));
    }

    let mut missing_components = Vec::new();
    let mut existing_ancestor = path;
    while !existing_ancestor.exists() {
        let name = existing_ancestor
            .file_name()
            .ok_or_else(|| OrbitError::InvalidInput("path has no file name".to_string()))?;
        missing_components.push(name.to_os_string());
        existing_ancestor = existing_ancestor.parent().ok_or_else(|| {
            OrbitError::InvalidInput("path has no existing parent directory".to_string())
        })?;
    }

    let mut canonical = existing_ancestor.canonicalize().map_err(|error| {
        OrbitError::Io(format!("failed to canonicalize parent directory: {error}"))
    })?;
    for component in missing_components.iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}
