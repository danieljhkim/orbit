//! Public `OrbitRuntime` surface for the project-learnings CLI subcommands.
//!
//! Mirrors the helpers used by `orbit.learning.*` MCP tools but lives on the
//! runtime so `orbit-cli` can call them without depending on the host-side
//! dispatch layer. Tool-host and CLI both reach into
//! `runtime.stores().learnings()`, which is the single source of truth.

use std::path::Path;

use orbit_common::types::{EvidenceKind, Learning, LearningStatus, NotFoundKind, OrbitError};
use orbit_store::{
    LearningCreateParams, LearningSearchParams, LearningSearchResult, LearningUpdateParams,
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
        self.stores().learnings().search(params)
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
