//! Public `OrbitRuntime` surface for the project-learnings CLI subcommands.
//!
//! Mirrors the helpers used by `orbit.learning.*` MCP tools but lives on the
//! runtime so `orbit-cli` can call them without depending on the host-side
//! dispatch layer. Tool-host and CLI both reach into
//! `runtime.stores().learnings()`, which is the single source of truth.

use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::{
    AuditEventStatus, EvidenceKind, Learning, LearningStatus, NotFoundKind, OrbitError,
    audit_execution_id,
};
use orbit_store::{
    AuditEventInsertParams, LEARNING_SHOWN_TARGET_TYPE, LearningCreateParams, LearningListEntry,
    LearningSearchParams, LearningSearchResult, LearningUpdateParams, LearningUsageStat,
    RemoteArtifactStub, learning_layout::LearningLayoutMigrationReport,
};
use serde::Deserialize;

use crate::OrbitRuntime;

#[derive(Debug, Deserialize)]
struct LearningConfigFile {
    learning: Option<LearningConfigSection>,
}

#[derive(Debug, Deserialize)]
struct LearningConfigSection {
    search: Option<LearningSearchConfigSection>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LearningSearchConfig {
    pub semantic_weight: f32,
}

impl Default for LearningSearchConfig {
    fn default() -> Self {
        Self {
            semantic_weight: 0.5,
        }
    }
}

#[derive(Debug, Deserialize)]
struct LearningSearchConfigSection {
    semantic_weight: Option<f32>,
}

impl OrbitRuntime {
    /// Verify that a multi-host owner finalizer is operating through the exact
    /// checkout-bound runtime capability selected by the broker. The caller
    /// supplies only the stable runtime workspace ID; checkout paths come from
    /// the trusted runtime binding, never from public request data or cwd.
    pub fn verify_preallocated_owner_runtime(
        &self,
        expected_runtime_workspace_id: &str,
    ) -> Result<(), OrbitError> {
        let binding = self.workspace_runtime_binding().ok_or_else(|| {
            OrbitError::InvalidInput(
                "preallocated knowledge finalization requires a registered exact-checkout runtime binding"
                    .to_string(),
            )
        })?;
        let persisted_workspace_id = self.workspace_id()?;
        if binding.workspace_id != expected_runtime_workspace_id
            || persisted_workspace_id != expected_runtime_workspace_id
        {
            return Err(OrbitError::InvalidInput(format!(
                "preallocated knowledge workspace mismatch: expected '{expected_runtime_workspace_id}', runtime binding is '{}', persisted checkout identity is '{persisted_workspace_id}'",
                binding.workspace_id
            )));
        }
        let bound_repo = binding.repo_root.canonicalize().map_err(|error| {
            OrbitError::InvalidInput(format!(
                "preallocated owner checkout '{}' is unavailable: {error}",
                binding.repo_root.display()
            ))
        })?;
        let runtime_repo = self.paths().repo_root.canonicalize().map_err(|error| {
            OrbitError::InvalidInput(format!(
                "runtime checkout '{}' is unavailable: {error}",
                self.paths().repo_root.display()
            ))
        })?;
        if bound_repo != runtime_repo {
            return Err(OrbitError::InvalidInput(format!(
                "preallocated owner checkout binding drifted: bound '{}', runtime '{}'",
                bound_repo.display(),
                runtime_repo.display()
            )));
        }
        Ok(())
    }

    pub fn create_learning(&self, params: LearningCreateParams) -> Result<Learning, OrbitError> {
        let learning = self.stores().learnings().add(params)?;
        self.record_id_allocation_audit("learning", &learning.id)?;
        Ok(learning)
    }

    pub fn get_learning(&self, id: &str) -> Result<Learning, OrbitError> {
        match self.stores().learnings().get_federated(id)? {
            Some(learning) => Ok(learning),
            None => {
                if let Some(stub) = self.stores().learnings().remote_stub(id)? {
                    return Err(remote_artifact_error("learning", &stub));
                }
                Err(OrbitError::not_found(
                    NotFoundKind::Learning,
                    id.to_string(),
                ))
            }
        }
    }

    pub fn list_learnings(
        &self,
        status: Option<LearningStatus>,
    ) -> Result<Vec<Learning>, OrbitError> {
        self.stores().learnings().list(status)
    }

    pub fn list_learning_entries(
        &self,
        status: Option<LearningStatus>,
        include_remote: bool,
    ) -> Result<Vec<LearningListEntry>, OrbitError> {
        self.stores()
            .learnings()
            .list_entries(status, include_remote)
    }

    pub fn search_learnings(
        &self,
        params: LearningSearchParams,
    ) -> Result<Vec<LearningSearchResult>, OrbitError> {
        let params = normalize_learning_search_params(&self.paths().repo_root, params)?;
        self.stores().learnings().search(params)
    }

    pub fn learning_search_config(&self) -> Result<LearningSearchConfig, OrbitError> {
        read_learning_search_config_from_config_path(&self.config_path())
    }

    /// Record a `learning_shown` audit event — the passive, ungameable usage
    /// signal emitted when an agent opens a learning's full body via
    /// `orbit learning show` (CLI or MCP). Keyed by learning ID + session.
    ///
    /// Fails open: an unavailable audit backend logs a warning and returns
    /// `Ok(())` so the show surface keeps working. The session id resolves
    /// from `ORBIT_SESSION_ID` (the injecting session exports it); an absent
    /// session records a `None`, which the rollup still counts per learning.
    pub fn record_learning_shown(&self, learning_id: &str) -> Result<(), OrbitError> {
        let session_id = std::env::var("ORBIT_SESSION_ID")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let working_directory = std::env::current_dir()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let params = AuditEventInsertParams {
            execution_id: audit_execution_id("learning"),
            command: "learning".to_string(),
            subcommand: Some("show".to_string()),
            tool_name: None,
            target_type: Some(LEARNING_SHOWN_TARGET_TYPE.to_string()),
            target_id: Some(learning_id.to_string()),
            role: "agent".to_string(),
            status: AuditEventStatus::Success,
            exit_code: 0,
            duration_ms: 0,
            working_directory,
            arguments_json: None,
            stdout_truncated: None,
            stderr_truncated: None,
            error_message: None,
            host: std::env::var("HOSTNAME").ok(),
            pid: std::process::id(),
            session_id,
            workspace_id: None,
            caller_machine_id: None,
            caller_host_id: None,
            process_machine_id: None,
            process_host_id: None,
            transport: None,
            effective_capabilities: Default::default(),
            origin_session_id: None,
            mcp_call_id: None,
            lease_id: None,
            task_id: std::env::var("ORBIT_TASK_ID")
                .ok()
                .filter(|value| !value.is_empty()),
            job_run_id: std::env::var("ORBIT_RUN_ID")
                .ok()
                .filter(|value| !value.is_empty()),
            activity_id: std::env::var("ORBIT_ACTIVITY_ID")
                .ok()
                .filter(|value| !value.is_empty()),
            step_index: std::env::var("ORBIT_STEP_INDEX")
                .ok()
                .and_then(|value| value.parse().ok()),
        };
        if let Err(error) = self.record_audit_event(&params) {
            tracing::warn!(
                learning_id,
                error = %error,
                "learning_shown audit emit failed open"
            );
        }
        Ok(())
    }

    /// Per-learning usage rollup over the global audit store: injection
    /// counts from `learning_injected` events, show counts from
    /// `learning_shown` events. Input for the deprecation sweep (ORB-10318).
    pub fn learning_usage_stats(
        &self,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<LearningUsageStat>, OrbitError> {
        self.stores().audit_events().learning_usage(since.as_ref())
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

    pub fn sync_learnings(&self) -> Result<(), OrbitError> {
        self.stores().learnings().sync()
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

pub fn parse_learning_search_config_from_config_toml(
    raw: &str,
) -> Result<LearningSearchConfig, OrbitError> {
    if raw.trim().is_empty() {
        return Ok(LearningSearchConfig::default());
    }
    let parsed = toml::from_str::<LearningConfigFile>(raw).map_err(|error| {
        OrbitError::InvalidInput(format!("invalid learning config in config.toml: {error}"))
    })?;
    let semantic_weight = parsed
        .learning
        .and_then(|section| section.search)
        .and_then(|section| section.semantic_weight)
        .unwrap_or_else(|| LearningSearchConfig::default().semantic_weight)
        .clamp(0.0, 1.0);
    Ok(LearningSearchConfig { semantic_weight })
}

fn read_learning_search_config_from_config_path(
    path: &Path,
) -> Result<LearningSearchConfig, OrbitError> {
    if !path.exists() {
        return Ok(LearningSearchConfig::default());
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
    parse_learning_search_config_from_config_toml(&raw)
}

fn remote_artifact_error(kind: &str, stub: &RemoteArtifactStub) -> OrbitError {
    OrbitError::Store(format!(
        "{kind} {} is recorded in another worktree and its body is not locally readable; worktree_root={}, branch={}",
        stub.id,
        stub.worktree_root.display(),
        stub.branch.as_deref().unwrap_or("<none>")
    ))
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
    match classify_learning_search_path(repo_root, path)? {
        LearningSearchPathScope::Relative => Ok(path.to_string()),
        LearningSearchPathScope::WorkspaceRelative(relative) => Ok(relative),
        LearningSearchPathScope::OutsideWorkspace => Err(OrbitError::InvalidInput(format!(
            "filesystem path `{path}` must stay inside the workspace root"
        ))),
    }
}

/// `pub` for the learning PreToolUse hook in `orbit-cmd` [ORB-10016].
pub fn learning_search_path_matches_workspace(
    repo_root: &Path,
    path: &str,
) -> Result<bool, OrbitError> {
    Ok(!matches!(
        classify_learning_search_path(repo_root, path)?,
        LearningSearchPathScope::OutsideWorkspace
    ))
}

enum LearningSearchPathScope {
    Relative,
    WorkspaceRelative(String),
    OutsideWorkspace,
}

fn classify_learning_search_path(
    repo_root: &Path,
    path: &str,
) -> Result<LearningSearchPathScope, OrbitError> {
    let trimmed = path.trim();
    let candidate = Path::new(trimmed);
    if !candidate.is_absolute() {
        return Ok(LearningSearchPathScope::Relative);
    }

    let canonical_repo_root = canonicalize_with_missing_tail(repo_root)?;
    let canonical_candidate = canonicalize_with_missing_tail(candidate)?;
    if let Ok(relative) = canonical_candidate.strip_prefix(&canonical_repo_root) {
        return Ok(LearningSearchPathScope::WorkspaceRelative(
            workspace_relative_path_string(relative),
        ));
    }

    if let Some(relative) =
        linked_worktree_relative_path(&canonical_repo_root, candidate, &canonical_candidate)
    {
        return Ok(LearningSearchPathScope::WorkspaceRelative(relative));
    }

    Ok(LearningSearchPathScope::OutsideWorkspace)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learning_search_config_defaults_and_clamps_semantic_weight() {
        assert_eq!(
            parse_learning_search_config_from_config_toml("")
                .unwrap()
                .semantic_weight,
            0.5
        );
        assert_eq!(
            parse_learning_search_config_from_config_toml(
                "[learning.search]\nsemantic_weight = 0.7\n"
            )
            .unwrap()
            .semantic_weight,
            0.7
        );
        assert_eq!(
            parse_learning_search_config_from_config_toml(
                "[learning.search]\nsemantic_weight = -1.0\n"
            )
            .unwrap()
            .semantic_weight,
            0.0
        );
        assert_eq!(
            parse_learning_search_config_from_config_toml(
                "[learning.search]\nsemantic_weight = 2.0\n"
            )
            .unwrap()
            .semantic_weight,
            1.0
        );
    }
}
