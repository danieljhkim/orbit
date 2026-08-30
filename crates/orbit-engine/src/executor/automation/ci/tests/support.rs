//! Scripted GitHub state.
//!
//! Collection is a pure function of what GitHub says, so the tests script that
//! end and never spawn `gh`.

use std::collections::HashMap;
use std::sync::Mutex;

use orbit_common::OrbitError;
use serde_json::{Value, json};

use super::super::query::{AuthStatus, CiQueries, LogScope, RunLog};

/// Scripted GitHub answers. Anything not scripted is an empty result, which is
/// itself a case worth exercising.
#[derive(Default)]
pub(super) struct FakeQueries {
    pub(super) auth: Option<AuthStatus>,
    pub(super) repo: Value,
    pub(super) pull_requests: Vec<Value>,
    pub(super) branch_heads: HashMap<String, String>,
    /// Runs per branch. A `Vec` of pages: each `runs_for_branch` call pops the
    /// next page, so a test can make CI progress between calls.
    pub(super) runs: Mutex<HashMap<String, Vec<Vec<Value>>>>,
    pub(super) run_views: HashMap<String, Value>,
    pub(super) logs: HashMap<(String, bool), String>,
}

impl FakeQueries {
    pub(super) fn authenticated() -> Self {
        Self {
            auth: Some(AuthStatus {
                available: true,
                authenticated: true,
                detail: "GitHub CLI is authenticated on this host".to_string(),
            }),
            repo: json!({
                "name": "orbit",
                "full_name": "acme/orbit",
                "default_branch": "main",
            }),
            ..Self::default()
        }
    }

    pub(super) fn unauthenticated(detail: &str) -> Self {
        Self {
            auth: Some(AuthStatus {
                available: true,
                authenticated: false,
                detail: detail.to_string(),
            }),
            ..Self::default()
        }
    }

    pub(super) fn with_head(mut self, branch: &str, sha: &str) -> Self {
        self.branch_heads
            .insert(branch.to_string(), sha.to_string());
        self
    }

    pub(super) fn with_runs(self, branch: &str, pages: Vec<Vec<Value>>) -> Self {
        self.runs
            .lock()
            .expect("runs lock")
            .insert(branch.to_string(), pages);
        self
    }

    pub(super) fn with_run_view(mut self, run_id: &str, view: Value) -> Self {
        self.run_views.insert(run_id.to_string(), view);
        self
    }

    pub(super) fn with_log(mut self, run_id: &str, all_scope: bool, log: &str) -> Self {
        self.logs
            .insert((run_id.to_string(), all_scope), log.to_string());
        self
    }
}

impl CiQueries for FakeQueries {
    fn auth_status(&self) -> AuthStatus {
        self.auth.clone().unwrap_or(AuthStatus {
            available: false,
            authenticated: false,
            detail: "no GitHub CLI on this host".to_string(),
        })
    }

    fn repo_view(&self) -> Result<Value, OrbitError> {
        Ok(self.repo.clone())
    }

    fn open_pull_requests(&self, limit: u64) -> Result<Vec<Value>, OrbitError> {
        Ok(self
            .pull_requests
            .iter()
            .take(limit as usize)
            .cloned()
            .collect())
    }

    fn runs_for_branch(&self, branch: &str, _limit: u64) -> Result<Vec<Value>, OrbitError> {
        let mut runs = self.runs.lock().expect("runs lock");
        let Some(pages) = runs.get_mut(branch) else {
            return Ok(Vec::new());
        };
        if pages.len() > 1 {
            Ok(pages.remove(0))
        } else {
            Ok(pages.first().cloned().unwrap_or_default())
        }
    }

    fn run_view(&self, run_id: &str) -> Result<Value, OrbitError> {
        Ok(self
            .run_views
            .get(run_id)
            .cloned()
            .unwrap_or_else(|| json!({"failed_jobs": []})))
    }

    fn run_logs(
        &self,
        run_id: &str,
        scope: LogScope,
        max_bytes: usize,
    ) -> Result<RunLog, OrbitError> {
        let raw = self
            .logs
            .get(&(run_id.to_string(), scope == LogScope::All))
            .cloned()
            .unwrap_or_default();
        Ok(super::super::query::bounded_run_log(&raw, max_bytes))
    }

    fn remote_branch_head(&self, branch: &str) -> Result<Option<String>, OrbitError> {
        Ok(self.branch_heads.get(branch).cloned())
    }
}

pub(super) fn run(
    run_id: u64,
    workflow: &str,
    sha: &str,
    status: &str,
    conclusion: Option<&str>,
    created_at: &str,
) -> Value {
    json!({
        "run_id": run_id,
        "workflow": workflow,
        "title": format!("{workflow} on {sha}"),
        "status": status,
        "conclusion": conclusion,
        "event": "push",
        "head_branch": "topic",
        "reported_head_sha": sha,
        "created_at": created_at,
        "url": format!("https://github.com/acme/orbit/actions/runs/{run_id}"),
    })
}
