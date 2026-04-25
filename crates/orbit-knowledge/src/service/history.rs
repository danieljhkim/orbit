//! Task-history query service (T20260421-0528).
//!
//! Resolves `orbit task history <selector>` queries against the branch-aware
//! knowledge graph. Prefers the graph-backed path (`task_ids` on the node +
//! sidecar index); falls back to a `git log` + regex scan when either is
//! missing.
//!
//! The service emits a structurally stable result across both paths so callers
//! can rely on a single JSON shape regardless of whether the graph is
//! available. Warnings (fallback, staleness) are surfaced on the caller side
//! via the `warnings` field and must be routed to stderr by the CLI.

use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Serialize;

use crate::error::KnowledgeError;
use crate::graph::object_store::{GraphObjectStore, RefName};
use crate::pipeline::history;
use crate::selector::Selector;
use crate::store::KnowledgeStore;
use crate::store::task_commits::{self, TaskCommitsIndex};

const TASK_ID_REGEX_STR: &str = r"\[T\d{8}-\d{4}(?:-\d+)?\]";

/// Default threshold (in commits) past which the staleness warning fires.
/// Overridable via `HistoryQueryOptions::staleness_threshold`.
pub const DEFAULT_STALENESS_THRESHOLD: u64 = 50;

#[derive(Debug, Clone)]
pub struct HistoryQueryOptions<'a> {
    pub knowledge_dir: &'a Path,
    pub repo_path: &'a Path,
    pub branch_ref: &'a RefName,
    pub selector: &'a Selector,
    pub staleness_threshold: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskHistoryResult {
    pub selector: String,
    pub source: HistorySource,
    pub task_history: Vec<TaskHistoryEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staleness: Option<StalenessInfo>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub structural_conflict: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistorySource {
    Graph,
    GitLogFallback,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskHistoryEntry {
    pub task_id: String,
    pub commits: Vec<CommitSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitSummary {
    pub sha: String,
    pub date: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StalenessInfo {
    pub commits_behind: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    pub head: String,
    pub remediation: String,
}

pub fn query_task_history(
    options: &HistoryQueryOptions,
) -> Result<TaskHistoryResult, KnowledgeError> {
    match try_graph_backed(options)? {
        GraphBackedOutcome::Hit(result) => Ok(result),
        GraphBackedOutcome::Fallback { reason } => fallback_git_log(options, Some(reason)),
    }
}

/// Internal result of the graph-backed attempt. Either the query succeeded
/// with a complete `TaskHistoryResult`, or a specific failure mode surfaced
/// and the caller should fall back to `git log` with the named reason.
enum GraphBackedOutcome {
    Hit(TaskHistoryResult),
    Fallback { reason: String },
}

fn try_graph_backed(options: &HistoryQueryOptions) -> Result<GraphBackedOutcome, KnowledgeError> {
    let knowledge_dir = options.knowledge_dir;
    if !knowledge_dir.is_dir() {
        return Ok(GraphBackedOutcome::Fallback {
            reason: format!(
                "knowledge graph directory not found at {} — falling back to `git log`",
                knowledge_dir.display()
            ),
        });
    }

    let default_ref = git_default_ref(options.repo_path);
    let store = match KnowledgeStore::open(
        knowledge_dir,
        options.branch_ref,
        None,
        default_ref.as_ref(),
    ) {
        Ok(store) => store,
        Err(error) => {
            return Ok(GraphBackedOutcome::Fallback {
                reason: format!(
                    "knowledge graph ref `{}` unavailable ({error}) — falling back to `git log`",
                    options.branch_ref
                ),
            });
        }
    };

    let info = match store.node_task_info(options.selector)? {
        Some(info) => info,
        None => {
            // Selector does not resolve in the loaded graph (typo, stale
            // graph, symbol extracted as a different kind than the selector
            // asked for, etc.). Fall back rather than silently returning an
            // empty result.
            return Ok(GraphBackedOutcome::Fallback {
                reason: format!(
                    "selector `{}` not found in the knowledge graph — falling back to `git log`",
                    options.selector
                ),
            });
        }
    };

    let sidecar_path = task_commits::sidecar_path(knowledge_dir, options.branch_ref.as_str());
    if !sidecar_path.is_file() {
        // Sidecar missing on this branch (e.g. rebuild has not yet run, or
        // user is on a non-default branch whose sidecar was never persisted).
        // Even if the node has populated task_ids, we have nothing to resolve
        // them against — fall back.
        return Ok(GraphBackedOutcome::Fallback {
            reason: format!(
                "task-commits sidecar missing at {} — falling back to `git log`; run \
                 `orbit task history rebuild` to repopulate",
                sidecar_path.display()
            ),
        });
    }
    let sidecar = task_commits::load(&sidecar_path)?;

    let mut warnings = Vec::new();
    let staleness = detect_staleness(options).unwrap_or_else(|error| {
        warnings.push(format!("staleness check skipped: {error}"));
        None
    });
    if let Some(staleness) = &staleness {
        warnings.push(format!(
            "knowledge graph is {} commits behind HEAD; run `{}`",
            staleness.commits_behind, staleness.remediation
        ));
    }

    let task_history = resolve_from_sidecar(&info.task_ids, &sidecar);

    Ok(GraphBackedOutcome::Hit(TaskHistoryResult {
        selector: options.selector.to_string(),
        source: HistorySource::Graph,
        task_history,
        staleness,
        structural_conflict: info.structural_conflict,
        warnings,
    }))
}

fn resolve_from_sidecar(task_ids: &[String], sidecar: &TaskCommitsIndex) -> Vec<TaskHistoryEntry> {
    task_ids
        .iter()
        .map(|task_id| {
            let commits = sidecar
                .get(task_id)
                .map(|entries| {
                    entries
                        .iter()
                        .map(|entry| CommitSummary {
                            sha: entry.sha.clone(),
                            date: entry.date.to_rfc3339(),
                            summary: entry.summary.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            TaskHistoryEntry {
                task_id: task_id.clone(),
                commits,
            }
        })
        .collect()
}

fn detect_staleness(
    options: &HistoryQueryOptions,
) -> Result<Option<StalenessInfo>, KnowledgeError> {
    let store = GraphObjectStore::new(options.knowledge_dir.join("graph"));
    let current_ref = match store.read_ref(options.branch_ref) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let Some(cursor) = current_ref.last_attributed_commit else {
        return Ok(None);
    };
    let Some(head) = history::resolve_head(options.repo_path)? else {
        return Ok(None);
    };
    if cursor == head {
        return Ok(None);
    }
    let gap = history::count_commits(options.repo_path, &cursor, &head)?;
    if gap <= options.staleness_threshold {
        return Ok(None);
    }
    Ok(Some(StalenessInfo {
        commits_behind: gap,
        cursor: Some(cursor),
        head,
        remediation: "orbit task history rebuild".to_string(),
    }))
}

fn fallback_git_log(
    options: &HistoryQueryOptions,
    precursor_reason: Option<String>,
) -> Result<TaskHistoryResult, KnowledgeError> {
    let mut warnings = Vec::new();
    if let Some(reason) = precursor_reason {
        warnings.push(reason);
    }
    warnings.push(
        "fallback mode: rename/move history is not available; results are derived \
         from `git log --` on the selector's file path"
            .to_string(),
    );
    if matches!(options.selector, Selector::Symbol { .. }) {
        warnings.push(
            "symbol selector: fallback widens to file-level history; \
             task IDs may include edits to unrelated symbols in the same file"
                .to_string(),
        );
    }

    let selector_paths = selector_paths(options.selector);
    let mut args: Vec<String> = vec![
        "log".into(),
        "--reverse".into(),
        "--topo-order".into(),
        "--format=%H%x00%cI%x00%s%x00%B%x1e".into(),
    ];
    if !selector_paths.is_empty() {
        args.push("--".into());
        for path in &selector_paths {
            args.push(path.to_string_lossy().into_owned());
        }
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    let output = orbit_common::utility::git::run_git(options.repo_path, &arg_refs)
        .map_err(|error| KnowledgeError::io(format!("git log failed: {error}")))?;
    let mut task_history: std::collections::BTreeMap<String, Vec<CommitSummary>> =
        std::collections::BTreeMap::new();
    if !output.success {
        warnings.push(format!(
            "git log returned non-zero exit status: {}",
            output.stderr.trim()
        ));
    } else {
        let regex = Regex::new(TASK_ID_REGEX_STR).expect("task-ID regex compiles");
        for record in output.stdout.split('\x1e') {
            let record = record.trim_matches('\n');
            if record.is_empty() {
                continue;
            }
            let parts: Vec<&str> = record.splitn(4, '\x00').collect();
            if parts.len() < 4 {
                continue;
            }
            let summary = CommitSummary {
                sha: parts[0].to_string(),
                date: parts[1].to_string(),
                summary: parts[2].to_string(),
            };
            for m in regex.find_iter(parts[3]) {
                let raw = m.as_str();
                let task_id = raw[1..raw.len() - 1].to_string();
                let entry = task_history.entry(task_id).or_default();
                if !entry.iter().any(|existing| existing.sha == summary.sha) {
                    entry.push(summary.clone());
                }
            }
        }
    }

    let history: Vec<TaskHistoryEntry> = task_history
        .into_iter()
        .map(|(task_id, commits)| TaskHistoryEntry { task_id, commits })
        .collect();

    Ok(TaskHistoryResult {
        selector: options.selector.to_string(),
        source: HistorySource::GitLogFallback,
        task_history: history,
        staleness: None,
        structural_conflict: false,
        warnings,
    })
}

fn selector_paths(selector: &Selector) -> Vec<PathBuf> {
    match selector {
        Selector::Dir { path } => vec![PathBuf::from(path.trim_end_matches('/'))],
        Selector::File { path } => vec![PathBuf::from(path)],
        Selector::Symbol { path, .. } => vec![PathBuf::from(path)],
    }
}

fn git_default_ref(repo_path: &Path) -> Option<RefName> {
    orbit_common::utility::git::default_branch(repo_path)
        .ok()
        .flatten()
        .and_then(|branch| RefName::new(branch).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use tempfile::tempdir;

    use crate::graph::nodes::{BaseNodeFields, CodebaseGraphV1, DirNode, FileNode};
    use crate::graph::object_store::{GraphObjectStore, RefName};
    use crate::selector::Selector;

    fn write_empty_manifest(dir: &Path) {
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"generated_at":"2026-04-21T00:00:00Z"}"#,
        )
        .unwrap();
    }

    fn base_node(id: &str, name: &str, location: &str, parent_id: Option<&str>) -> BaseNodeFields {
        BaseNodeFields {
            id: id.to_string(),
            identity_key: id.to_string(),
            object_hash: None,
            name: name.to_string(),
            location: location.to_string(),
            language: "rust".to_string(),
            description: String::new(),
            parent_id: parent_id.map(ToOwned::to_owned),
            is_locked: false,
            lineage_locked: false,
            lock_owner: None,
            lock_reason: String::new(),
            task_ids: Vec::new(),
            structural_conflict: false,
        }
    }

    /// Build a minimal graph with one file node that already carries
    /// `task_ids`. Skips leaves and nested directories to avoid exercising the
    /// pre-existing `dir references missing dir child` pipeline bug.
    fn write_fixture_graph(
        knowledge_dir: &Path,
        branch: &RefName,
        file_task_ids: Vec<String>,
    ) -> (String, String) {
        std::fs::create_dir_all(knowledge_dir).unwrap();
        write_empty_manifest(knowledge_dir);

        let root_id = "dir:.".to_string();
        let file_id = "file:src/lib.rs".to_string();

        let mut file_base = base_node(&file_id, "lib.rs", "src/lib.rs", Some(&root_id));
        file_base.task_ids = file_task_ids;

        let graph = CodebaseGraphV1 {
            root_dir_id: root_id.clone(),
            dirs: vec![DirNode {
                base: base_node(&root_id, ".", ".", None),
                dir_children: Vec::new(),
                file_children: vec![file_id.clone()],
            }],
            files: vec![FileNode {
                base: file_base,
                extension: Some("rs".to_string()),
                source_blob_hash: None,
                imports: Vec::new(),
                exports: Vec::new(),
                re_exports: Vec::new(),
                leaf_children: Vec::new(),
            }],
            leaves: Vec::new(),
        };

        let store = GraphObjectStore::new(knowledge_dir.join("graph"));
        store.prepare_refs_layout(Some(branch)).unwrap();
        let current_ref = store.write_graph(&graph).unwrap();
        store.write_ref_atomic(branch, &current_ref).unwrap();
        (root_id, file_id)
    }

    fn make_options<'a>(
        knowledge_dir: &'a Path,
        repo_path: &'a Path,
        branch_ref: &'a RefName,
        selector: &'a Selector,
    ) -> HistoryQueryOptions<'a> {
        HistoryQueryOptions {
            knowledge_dir,
            repo_path,
            branch_ref,
            selector,
            staleness_threshold: DEFAULT_STALENESS_THRESHOLD,
        }
    }

    #[test]
    fn selector_paths_for_dir_trims_trailing_slash() {
        let s = Selector::Dir {
            path: "src/foo/".to_string(),
        };
        assert_eq!(selector_paths(&s), vec![PathBuf::from("src/foo")]);
    }

    #[test]
    fn selector_paths_for_file() {
        let s = Selector::File {
            path: "src/foo.rs".to_string(),
        };
        assert_eq!(selector_paths(&s), vec![PathBuf::from("src/foo.rs")]);
    }

    #[test]
    fn selector_paths_for_symbol_uses_file_path() {
        let s = Selector::Symbol {
            path: "src/foo.rs".to_string(),
            symbol: "bar".to_string(),
            kind: "function".to_string(),
        };
        assert_eq!(selector_paths(&s), vec![PathBuf::from("src/foo.rs")]);
    }

    #[test]
    fn resolve_from_sidecar_returns_empty_commits_for_unknown_task_id() {
        let sidecar = TaskCommitsIndex::default();
        let task_ids = vec!["T20260421-0528".to_string()];
        let history = resolve_from_sidecar(&task_ids, &sidecar);
        assert_eq!(history.len(), 1);
        assert!(history[0].commits.is_empty());
    }

    #[test]
    fn resolve_from_sidecar_maps_task_ids_to_commits() {
        let mut sidecar = TaskCommitsIndex::default();
        let date: DateTime<Utc> = DateTime::parse_from_rfc3339("2026-04-21T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        sidecar.append(
            "T20260421-0528".to_string(),
            crate::store::task_commits::CommitSummary {
                sha: "abc".to_string(),
                date,
                summary: "add task_ids".to_string(),
            },
        );
        let task_ids = vec!["T20260421-0528".to_string()];
        let history = resolve_from_sidecar(&task_ids, &sidecar);
        assert_eq!(history[0].commits.len(), 1);
        assert_eq!(history[0].commits[0].sha, "abc");
    }

    fn assert_fallback(outcome: GraphBackedOutcome) -> String {
        match outcome {
            GraphBackedOutcome::Fallback { reason } => reason,
            GraphBackedOutcome::Hit(_) => panic!("expected fallback, got hit"),
        }
    }

    #[test]
    fn try_graph_backed_falls_back_when_knowledge_dir_missing() {
        let dir = tempdir().unwrap();
        let knowledge_dir = dir.path().join("nope");
        let repo = tempdir().unwrap();
        let branch = RefName::new("main").unwrap();
        let selector = Selector::File {
            path: "src/foo.rs".to_string(),
        };
        let options = make_options(&knowledge_dir, repo.path(), &branch, &selector);
        let reason = assert_fallback(try_graph_backed(&options).unwrap());
        assert!(
            reason.contains("knowledge graph directory not found"),
            "unexpected reason: {reason}"
        );
    }

    #[test]
    fn try_graph_backed_falls_back_when_store_open_fails() {
        let dir = tempdir().unwrap();
        let knowledge_dir = dir.path().join("knowledge");
        std::fs::create_dir_all(&knowledge_dir).unwrap();
        write_empty_manifest(&knowledge_dir);
        let repo = tempdir().unwrap();
        let branch = RefName::new("does-not-exist").unwrap();
        let selector = Selector::File {
            path: "src/foo.rs".to_string(),
        };
        let options = make_options(&knowledge_dir, repo.path(), &branch, &selector);
        let reason = assert_fallback(try_graph_backed(&options).unwrap());
        assert!(
            reason.contains("knowledge graph ref"),
            "unexpected reason: {reason}"
        );
    }

    #[test]
    fn fallback_warns_about_symbol_precision_loss() {
        let repo = tempdir().unwrap();
        // Avoid needing a real git repo — point at a missing path. The
        // fallback will invoke git log and fail cleanly; we only care that the
        // warnings list includes the symbol-precision notice before git runs.
        let selector = Selector::Symbol {
            path: "src/foo.rs".to_string(),
            symbol: "bar".to_string(),
            kind: "function".to_string(),
        };
        let branch = RefName::new("main").unwrap();
        let knowledge_dir = repo.path().join("knowledge");
        let options = make_options(&knowledge_dir, repo.path(), &branch, &selector);
        let result = fallback_git_log(&options, Some("test precursor".to_string()))
            .unwrap_or_else(|error| panic!("fallback should not error: {error}"));
        let warnings: Vec<&str> = result.warnings.iter().map(String::as_str).collect();
        assert!(
            warnings.iter().any(|w| w.contains("test precursor")),
            "missing precursor reason in {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("rename/move history")),
            "missing rename/move notice in {warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("symbol selector") && w.contains("file-level history")),
            "missing symbol precision notice in {warnings:?}"
        );
    }

    #[test]
    fn fallback_omits_symbol_warning_for_file_selector() {
        let repo = tempdir().unwrap();
        let selector = Selector::File {
            path: "src/foo.rs".to_string(),
        };
        let branch = RefName::new("main").unwrap();
        let knowledge_dir = repo.path().join("knowledge");
        let options = make_options(&knowledge_dir, repo.path(), &branch, &selector);
        let result = fallback_git_log(&options, None).unwrap();
        assert!(
            !result
                .warnings
                .iter()
                .any(|w| w.contains("symbol selector")),
            "file selectors should not emit symbol-precision warnings"
        );
    }

    #[test]
    fn try_graph_backed_falls_back_when_sidecar_missing_even_if_node_has_task_ids() {
        let knowledge_root = tempdir().unwrap();
        let knowledge_dir = knowledge_root.path().join("knowledge");
        let repo = tempdir().unwrap();
        let branch = RefName::new("main").unwrap();
        write_fixture_graph(&knowledge_dir, &branch, vec!["T20260421-0528".to_string()]);

        // Deliberately do NOT write the sidecar. Even though the node has
        // populated task_ids, the fallback must still fire (P1 review fix).
        let selector = Selector::File {
            path: "src/lib.rs".to_string(),
        };
        let options = make_options(&knowledge_dir, repo.path(), &branch, &selector);
        let reason = assert_fallback(try_graph_backed(&options).unwrap());
        assert!(
            reason.contains("task-commits sidecar missing"),
            "unexpected reason: {reason}"
        );
    }

    #[test]
    fn try_graph_backed_falls_back_when_selector_unresolved() {
        let knowledge_root = tempdir().unwrap();
        let knowledge_dir = knowledge_root.path().join("knowledge");
        let repo = tempdir().unwrap();
        let branch = RefName::new("main").unwrap();
        write_fixture_graph(&knowledge_dir, &branch, Vec::new());

        // Selector for a file that does not exist in the graph.
        let selector = Selector::File {
            path: "src/does_not_exist.rs".to_string(),
        };
        let options = make_options(&knowledge_dir, repo.path(), &branch, &selector);
        let reason = assert_fallback(try_graph_backed(&options).unwrap());
        assert!(
            reason.contains("not found in the knowledge graph"),
            "unexpected reason: {reason}"
        );
    }

    #[test]
    fn try_graph_backed_hit_when_everything_present() {
        let knowledge_root = tempdir().unwrap();
        let knowledge_dir = knowledge_root.path().join("knowledge");
        let repo = tempdir().unwrap();
        let branch = RefName::new("main").unwrap();
        write_fixture_graph(&knowledge_dir, &branch, vec!["T20260421-0528".to_string()]);

        // Write an empty-but-present sidecar so the guard is satisfied.
        let sidecar_path = task_commits::sidecar_path(&knowledge_dir, branch.as_str());
        task_commits::save(&sidecar_path, &TaskCommitsIndex::default()).unwrap();

        let selector = Selector::File {
            path: "src/lib.rs".to_string(),
        };
        let options = make_options(&knowledge_dir, repo.path(), &branch, &selector);
        match try_graph_backed(&options).unwrap() {
            GraphBackedOutcome::Hit(result) => {
                assert_eq!(result.task_history.len(), 1);
                assert_eq!(result.task_history[0].task_id, "T20260421-0528");
                // Sidecar is empty so commits list is empty; graph path still
                // succeeds — the node's task_ids is the source of truth.
                assert!(result.task_history[0].commits.is_empty());
            }
            GraphBackedOutcome::Fallback { reason } => {
                panic!("expected hit, got fallback: {reason}")
            }
        }
    }

    #[test]
    fn fallback_includes_precursor_reason_when_provided() {
        let repo = tempdir().unwrap();
        let selector = Selector::File {
            path: "src/foo.rs".to_string(),
        };
        let branch = RefName::new("main").unwrap();
        let knowledge_dir = repo.path().join("knowledge");
        let options = make_options(&knowledge_dir, repo.path(), &branch, &selector);
        let result = fallback_git_log(&options, Some("custom reason".to_string())).unwrap();
        assert_eq!(result.warnings[0], "custom reason");
    }
}
