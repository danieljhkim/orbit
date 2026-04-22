//! Attribution pipeline stage (T20260421-0528).
//!
//! Runs after `build_graph_leaves` and before `persist_graph`. Walks the commit
//! DAG from the per-branch `last_attributed_commit` cursor to HEAD, parses
//! `[T...]` tags from each commit message, and attributes them to:
//!
//! - The file node at the current-tree path of each changed file.
//! - The containing directory node (rollup).
//! - Leaf nodes whose current-tree range is touched by any of the commit's
//!   hunks, matched to symbols at the commit's tree via the identity matcher
//!   (rules 1+2; no body-similarity heuristics).
//!
//! Also writes the `task-commits` sidecar index keyed by task ID and records
//! a per-commit touched-node-id map. The map is reused by the merge-aware
//! `structural_conflict` detection (Phase 4).
//!
//! The operation-log read hook (`.orbit/operations/<sha>.json`) is a Phase 6
//! extension point; a stub check lives here but currently always falls through
//! to the matcher.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing::debug;

use crate::error::KnowledgeError;
use crate::extract::{ExtractorRegistry, FileKind};
use crate::graph::nodes::{CodebaseGraphV1, LeafKind, LeafNode};
use crate::graph::object_store::GraphObjectStore;
use crate::pipeline::context::PipelineContext;
use crate::pipeline::history::{self, ChangeKind, CommitInfo};
use crate::store::task_commits::{CommitSummary, TaskCommitsIndex};

/// Outputs of the attribution pass. Consumed downstream by persist.
#[derive(Debug, Default)]
pub struct AttributeOutcome {
    pub sidecar: TaskCommitsIndex,
    pub head_sha: Option<String>,
    /// Per-commit set of current-tree node IDs touched by that commit. Keys
    /// are commit SHAs. Populated for every commit visited by the walker
    /// (including commits without task IDs). Surfaces primarily for tests and
    /// diagnostics — downstream consumers read `task_ids` off the nodes.
    pub touched_by_commit: HashMap<String, HashSet<String>>,
    /// `true` when the walker ran from the empty cursor (fresh or first-time
    /// post-upgrade backfill). Surfaced so callers can describe the workload.
    pub was_full_backfill: bool,
}

/// Entry point. Mutates `ctx.graph` in place, appending `task_ids` to every
/// touched node and marking `structural_conflict` on nodes touched by both
/// parents of any merge commit in the walk. Returns the sidecar + touched-map.
pub fn attribute_history(ctx: &mut PipelineContext) -> Result<AttributeOutcome, KnowledgeError> {
    let head = history::resolve_head(&ctx.repo_path)?;
    let Some(head_sha) = head else {
        return Ok(AttributeOutcome::default());
    };

    let cursor = read_previous_cursor(ctx);
    let was_full_backfill = cursor.is_none();

    // Hydrate task_ids and structural_conflict from the previous ref so that a
    // repeat rebuild on the same HEAD (cursor == HEAD → no commits walked) is
    // byte-identical to the first rebuild. Without this, task_ids set in an
    // earlier rebuild would be dropped when `build_graph_*` produces fresh
    // nodes in ctx.graph.
    hydrate_previous_attributions(ctx);

    let commits = history::walk_commits(&ctx.repo_path, cursor.as_deref(), &head_sha)?;

    if commits.is_empty() {
        return Ok(AttributeOutcome {
            head_sha: Some(head_sha),
            was_full_backfill,
            ..AttributeOutcome::default()
        });
    }

    let matcher = IdentityMatcher::new(&ctx.graph.leaves);
    let path_to_file_idx: HashMap<String, usize> = ctx
        .graph
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.base.location.clone(), i))
        .collect();
    let dir_by_loc: HashMap<String, usize> = ctx
        .graph
        .dirs
        .iter()
        .enumerate()
        .map(|(i, d)| (d.base.location.clone(), i))
        .collect();

    let registry = ExtractorRegistry::new();
    let operations_dir = operation_log_dir(&ctx.output_dir);

    // Determine which commits are handled by operation logs rather than the
    // matcher. For T20260421-0528 only the read-side hook is reserved; a valid
    // log (top-level `operations: []`) short-circuits both the matcher and the
    // sidecar for that commit. Malformed logs emit a stderr warning and fall
    // back to the matcher.
    let mut operation_log_skipped: HashSet<String> = HashSet::new();
    for commit in &commits {
        match inspect_operation_log(&operations_dir, &commit.sha) {
            OperationLogStatus::Absent => {}
            OperationLogStatus::Valid => {
                operation_log_skipped.insert(commit.sha.clone());
            }
            OperationLogStatus::Malformed(error) => {
                eprintln!(
                    "warning: malformed operation log for {}: {error} — falling back to matcher",
                    commit.sha
                );
            }
        }
    }

    // Phase A (read-only): compute touched node IDs per commit.
    let mut touched_by_commit: HashMap<String, HashSet<String>> = HashMap::new();
    for commit in &commits {
        if operation_log_skipped.contains(&commit.sha) {
            touched_by_commit.insert(commit.sha.clone(), HashSet::new());
            continue;
        }
        let touched = collect_touched_for_commit(
            &ctx.repo_path,
            &ctx.graph,
            &path_to_file_idx,
            &dir_by_loc,
            &matcher,
            &registry,
            commit,
        );
        touched_by_commit.insert(commit.sha.clone(), touched);
    }

    // Phase B (write): apply task_ids to touched nodes and populate the sidecar.
    let mut sidecar = TaskCommitsIndex::default();
    for commit in &commits {
        if commit.task_ids.is_empty() {
            continue;
        }
        if operation_log_skipped.contains(&commit.sha) {
            continue;
        }
        for task_id in &commit.task_ids {
            sidecar.append(
                task_id.clone(),
                CommitSummary {
                    sha: commit.sha.clone(),
                    date: commit.date,
                    summary: commit.summary.clone(),
                },
            );
        }
        let Some(touched_ids) = touched_by_commit.get(&commit.sha) else {
            continue;
        };
        apply_task_ids_to_nodes(&mut ctx.graph, touched_ids, &commit.task_ids);
    }

    sort_and_dedup_task_ids(&mut ctx.graph);
    sidecar.finalize();

    // Phase C (read-only): compute structural-conflict node IDs across merges
    // in the walked range. Ancestry chains may extend before the cursor; cache
    // misses fall through to `history::commit_info` to compute touched sets
    // on demand.
    let conflict_ids = detect_structural_conflict_ids(
        &ctx.repo_path,
        &ctx.graph,
        &commits,
        &mut touched_by_commit,
        &path_to_file_idx,
        &dir_by_loc,
        &matcher,
        &registry,
    )?;

    // Phase D (write): apply structural_conflict marks.
    apply_structural_conflict(&mut ctx.graph, &conflict_ids);

    Ok(AttributeOutcome {
        sidecar,
        head_sha: Some(head_sha),
        touched_by_commit,
        was_full_backfill,
    })
}

/// Pure function: walk a commit's changed files and return the set of current-
/// tree node IDs it touches. No mutation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn collect_touched_for_commit(
    repo_path: &Path,
    graph: &CodebaseGraphV1,
    path_to_file_idx: &HashMap<String, usize>,
    dir_by_loc: &HashMap<String, usize>,
    matcher: &IdentityMatcher,
    registry: &ExtractorRegistry,
    commit: &CommitInfo,
) -> HashSet<String> {
    let mut touched: HashSet<String> = HashSet::new();

    for file_diff in &commit.files_changed {
        let path_str = file_diff.path.to_string_lossy().into_owned();
        let Some(&file_idx) = path_to_file_idx.get(&path_str) else {
            continue;
        };

        touched.insert(graph.files[file_idx].base.id.clone());

        let dir_loc = dir_location_for_file(&path_str);
        if let Some(&dir_idx) = dir_by_loc.get(&dir_loc) {
            touched.insert(graph.dirs[dir_idx].base.id.clone());
        }

        if matches!(file_diff.change_kind, ChangeKind::Deleted) {
            continue;
        }

        let extension = file_diff
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");
        let file_kind = FileKind::from_extension(extension);
        if !file_kind.is_extractable() {
            continue;
        }
        let Some(extractor) = registry.get(file_kind) else {
            continue;
        };

        let source = match history::show_file_at_commit(repo_path, &commit.sha, &file_diff.path) {
            Ok(Some(source)) => source,
            Ok(None) => continue,
            Err(error) => {
                debug!(
                    commit = %commit.sha,
                    path = %path_str,
                    error = %error,
                    "failed to read historical file; skipping symbol-level attribution"
                );
                continue;
            }
        };

        // Extractors may panic on malformed historical trees. Protect the
        // rebuild from a single bad commit.
        let extract_result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| extractor.extract(&source)));
        let extracted = match extract_result {
            Ok(result) => result,
            Err(_) => {
                debug!(
                    commit = %commit.sha,
                    path = %path_str,
                    "extractor panicked on historical commit; attributing at file level only"
                );
                continue;
            }
        };

        for leaf in &extracted.leaves {
            let touches = file_diff
                .hunks
                .iter()
                .any(|hunk| hunk.touches_range(leaf.start_line as u32, leaf.end_line as u32));
            if !touches {
                continue;
            }
            if let Some(idx) = matcher.match_leaf(&path_str, &leaf.qualified_name, &leaf.kind) {
                touched.insert(graph.leaves[idx].base.id.clone());
            }
        }
    }

    touched
}

fn apply_task_ids_to_nodes(
    graph: &mut CodebaseGraphV1,
    touched_ids: &HashSet<String>,
    task_ids: &[String],
) {
    for dir in graph.dirs.iter_mut() {
        if touched_ids.contains(&dir.base.id) {
            append_task_ids(&mut dir.base.task_ids, task_ids);
        }
    }
    for file in graph.files.iter_mut() {
        if touched_ids.contains(&file.base.id) {
            append_task_ids(&mut file.base.task_ids, task_ids);
        }
    }
    for leaf in graph.leaves.iter_mut() {
        if touched_ids.contains(&leaf.base.id) {
            append_task_ids(&mut leaf.base.task_ids, task_ids);
        }
    }
}

/// Compute node IDs where both parents of a merge commit touched the node.
/// Ancestry chains may extend before the walker's cursor; cache misses fall
/// through to `history::commit_info`.
#[allow(clippy::too_many_arguments)]
fn detect_structural_conflict_ids(
    repo_path: &Path,
    graph: &CodebaseGraphV1,
    commits: &[CommitInfo],
    touched_by_commit: &mut HashMap<String, HashSet<String>>,
    path_to_file_idx: &HashMap<String, usize>,
    dir_by_loc: &HashMap<String, usize>,
    matcher: &IdentityMatcher,
    registry: &ExtractorRegistry,
) -> Result<HashSet<String>, KnowledgeError> {
    let mut conflict_ids: HashSet<String> = HashSet::new();

    for commit in commits {
        if commit.parents.len() != 2 {
            continue;
        }
        let parent_a = &commit.parents[0];
        let parent_b = &commit.parents[1];
        let Some(base) = history::merge_base(repo_path, parent_a, parent_b)? else {
            continue;
        };

        let chain_a = history::rev_list_range(repo_path, &base, parent_a)?;
        let chain_b = history::rev_list_range(repo_path, &base, parent_b)?;

        let touched_a = union_touched_along_chain(
            repo_path,
            graph,
            &chain_a,
            touched_by_commit,
            path_to_file_idx,
            dir_by_loc,
            matcher,
            registry,
        )?;
        let touched_b = union_touched_along_chain(
            repo_path,
            graph,
            &chain_b,
            touched_by_commit,
            path_to_file_idx,
            dir_by_loc,
            matcher,
            registry,
        )?;

        for id in touched_a.intersection(&touched_b) {
            conflict_ids.insert(id.clone());
        }
    }

    Ok(conflict_ids)
}

#[allow(clippy::too_many_arguments)]
fn union_touched_along_chain(
    repo_path: &Path,
    graph: &CodebaseGraphV1,
    chain: &[String],
    touched_by_commit: &mut HashMap<String, HashSet<String>>,
    path_to_file_idx: &HashMap<String, usize>,
    dir_by_loc: &HashMap<String, usize>,
    matcher: &IdentityMatcher,
    registry: &ExtractorRegistry,
) -> Result<HashSet<String>, KnowledgeError> {
    let mut union: HashSet<String> = HashSet::new();
    for sha in chain {
        if let Some(cached) = touched_by_commit.get(sha) {
            union.extend(cached.iter().cloned());
            continue;
        }
        // Cache miss — commit is before the walker's cursor. Fetch + compute.
        let commit = history::commit_info(repo_path, sha)?;
        let touched = collect_touched_for_commit(
            repo_path,
            graph,
            path_to_file_idx,
            dir_by_loc,
            matcher,
            registry,
            &commit,
        );
        union.extend(touched.iter().cloned());
        touched_by_commit.insert(sha.clone(), touched);
    }
    Ok(union)
}

fn apply_structural_conflict(graph: &mut CodebaseGraphV1, conflict_ids: &HashSet<String>) {
    if conflict_ids.is_empty() {
        return;
    }
    for dir in graph.dirs.iter_mut() {
        if conflict_ids.contains(&dir.base.id) {
            dir.base.structural_conflict = true;
        }
    }
    for file in graph.files.iter_mut() {
        if conflict_ids.contains(&file.base.id) {
            file.base.structural_conflict = true;
        }
    }
    for leaf in graph.leaves.iter_mut() {
        if conflict_ids.contains(&leaf.base.id) {
            leaf.base.structural_conflict = true;
        }
    }
}

/// Identity matcher (T20260421-0528 rules 1+2).
///
/// - Rule 1: exact match on `(path, qualified_name, kind)`.
/// - Rule 2: match on `(path, qualified_name)` when `kind` differs (e.g., `fn`
///   → `async fn`, `struct` → `enum`).
///
/// No body-similarity, no heuristics. Unmatched symbols produce no attribution
/// (the symbol is considered new at the current tree).
pub(crate) struct IdentityMatcher {
    /// `(file_path, qualified_name, kind_string)` → leaf index.
    by_kind: HashMap<(String, String, String), usize>,
    /// `(file_path, qualified_name)` → ordered list of leaf indices sharing
    /// that path+name but differing on kind.
    by_path_name: HashMap<(String, String), Vec<usize>>,
}

impl IdentityMatcher {
    pub(crate) fn new(leaves: &[LeafNode]) -> Self {
        let mut by_kind = HashMap::new();
        let mut by_path_name: HashMap<(String, String), Vec<usize>> = HashMap::new();

        for (idx, leaf) in leaves.iter().enumerate() {
            let (file_path, qualified_name) = split_leaf_location(&leaf.base.location);
            let kind_str = leaf_kind_to_str(&leaf.kind);
            by_kind.insert(
                (
                    file_path.clone(),
                    qualified_name.clone(),
                    kind_str.to_string(),
                ),
                idx,
            );
            by_path_name
                .entry((file_path, qualified_name))
                .or_default()
                .push(idx);
        }

        Self {
            by_kind,
            by_path_name,
        }
    }

    pub(crate) fn match_leaf(&self, path: &str, qualified_name: &str, kind: &str) -> Option<usize> {
        let key1 = (
            path.to_string(),
            qualified_name.to_string(),
            kind.to_string(),
        );
        if let Some(&idx) = self.by_kind.get(&key1) {
            return Some(idx);
        }
        let key2 = (path.to_string(), qualified_name.to_string());
        self.by_path_name
            .get(&key2)
            .and_then(|candidates| candidates.first().copied())
    }
}

fn split_leaf_location(location: &str) -> (String, String) {
    match location.split_once('#') {
        Some((path, qname)) => (path.to_string(), qname.to_string()),
        None => (location.to_string(), String::new()),
    }
}

fn leaf_kind_to_str(kind: &LeafKind) -> String {
    // LeafKind's Display matches the extractor's lowercase kind strings.
    kind.to_string()
}

fn dir_location_for_file(file_path: &str) -> String {
    let parent = Path::new(file_path)
        .parent()
        .map(|p| {
            if p.as_os_str().is_empty() {
                ".".to_string()
            } else {
                p.to_string_lossy().into_owned()
            }
        })
        .unwrap_or_else(|| ".".to_string());
    format!("{parent}/")
}

fn append_task_ids(dest: &mut Vec<String>, src: &[String]) {
    for id in src {
        if !dest.iter().any(|existing| existing == id) {
            dest.push(id.clone());
        }
    }
}

fn sort_and_dedup_task_ids(graph: &mut CodebaseGraphV1) {
    for node in graph.dirs.iter_mut() {
        node.base.task_ids.sort();
        node.base.task_ids.dedup();
    }
    for node in graph.files.iter_mut() {
        node.base.task_ids.sort();
        node.base.task_ids.dedup();
    }
    for node in graph.leaves.iter_mut() {
        node.base.task_ids.sort();
        node.base.task_ids.dedup();
    }
}

/// Result of inspecting `.orbit/operations/<sha>.json` for a commit.
///
/// Phase 6 of T20260421-0528 reserves the read-side hook without defining the
/// full operation schema (that is T20260421-0543's design task). A file with
/// top-level `operations: []` array is accepted as valid-but-noop; malformed
/// files fall back to the matcher with a stderr warning.
#[derive(Debug)]
enum OperationLogStatus {
    Absent,
    Valid,
    Malformed(String),
}

#[derive(Deserialize)]
struct OperationLogSkeleton {
    #[serde(default)]
    #[allow(dead_code)]
    operations: Vec<serde_json::Value>,
}

/// Path under which `operations/<sha>.json` files live. Defaults to
/// `<orbit_root>/operations/` where `orbit_root = knowledge_dir.parent()`.
pub(crate) fn operation_log_dir(knowledge_dir: &Path) -> PathBuf {
    knowledge_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("operations")
}

fn inspect_operation_log(operations_dir: &Path, sha: &str) -> OperationLogStatus {
    let path = operations_dir.join(format!("{sha}.json"));
    if !path.is_file() {
        return OperationLogStatus::Absent;
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) => return OperationLogStatus::Malformed(error.to_string()),
    };
    match serde_json::from_str::<OperationLogSkeleton>(&raw) {
        Ok(_) => OperationLogStatus::Valid,
        Err(error) => OperationLogStatus::Malformed(error.to_string()),
    }
}

fn read_previous_cursor(ctx: &PipelineContext) -> Option<String> {
    let store = GraphObjectStore::new(ctx.graph_dir());
    store
        .read_ref(&ctx.ref_name)
        .ok()
        .and_then(|cr| cr.last_attributed_commit)
}

/// Copy `task_ids` and `structural_conflict` from the previously-persisted
/// graph (if any) onto the freshly-built `ctx.graph`, keyed by node ID.
///
/// This guarantees `make idempotent` rebuild semantics: when the commit walker
/// produces zero new commits (cursor already at HEAD), the re-serialised graph
/// preserves the attribution state from the last rebuild rather than zeroing
/// it out. Without this pass, every no-op rebuild would drop every
/// previously-attributed task ID.
///
/// A node is matched only when its current-tree ID (derived from the identity
/// key) equals the ID stored in the previous graph — consistent with the
/// identity-matcher's "unchanged path+qname+kind" contract.
fn hydrate_previous_attributions(ctx: &mut PipelineContext) {
    let store = GraphObjectStore::new(ctx.graph_dir());
    let previous = match store.read_graph(&ctx.ref_name, None, ctx.default_ref_name.as_ref()) {
        Ok(graph) => graph,
        Err(_) => return,
    };

    let mut inherited: HashMap<String, (Vec<String>, bool)> = HashMap::new();
    for dir in &previous.dirs {
        inherited.insert(
            dir.base.id.clone(),
            (dir.base.task_ids.clone(), dir.base.structural_conflict),
        );
    }
    for file in &previous.files {
        inherited.insert(
            file.base.id.clone(),
            (file.base.task_ids.clone(), file.base.structural_conflict),
        );
    }
    for leaf in &previous.leaves {
        inherited.insert(
            leaf.base.id.clone(),
            (leaf.base.task_ids.clone(), leaf.base.structural_conflict),
        );
    }

    if inherited.is_empty() {
        return;
    }

    for dir in ctx.graph.dirs.iter_mut() {
        if let Some((task_ids, structural_conflict)) = inherited.get(&dir.base.id) {
            dir.base.task_ids = task_ids.clone();
            dir.base.structural_conflict = *structural_conflict;
        }
    }
    for file in ctx.graph.files.iter_mut() {
        if let Some((task_ids, structural_conflict)) = inherited.get(&file.base.id) {
            file.base.task_ids = task_ids.clone();
            file.base.structural_conflict = *structural_conflict;
        }
    }
    for leaf in ctx.graph.leaves.iter_mut() {
        if let Some((task_ids, structural_conflict)) = inherited.get(&leaf.base.id) {
            leaf.base.task_ids = task_ids.clone();
            leaf.base.structural_conflict = *structural_conflict;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::nodes::{BaseNodeFields, DirNode, FileNode, LeafKind, LeafNode};
    use tempfile::tempdir;

    fn leaf(location: &str, name: &str, kind: LeafKind) -> LeafNode {
        LeafNode {
            base: BaseNodeFields {
                id: format!("symbol:{location}:{kind}"),
                identity_key: String::new(),
                object_hash: None,
                name: name.to_string(),
                location: location.to_string(),
                language: "rust".to_string(),
                description: String::new(),
                parent_id: None,
                is_locked: false,
                lineage_locked: false,
                lock_owner: None,
                lock_reason: String::new(),
                task_ids: Vec::new(),
                structural_conflict: false,
            },
            kind,
            source: String::new(),
            source_blob_hash: None,
            source_hash: None,
            file_hash_at_capture: None,
            history: Vec::new(),
            input_signature: Vec::new(),
            output_signature: Vec::new(),
            start_line: Some(1),
            end_line: Some(10),
            children: Vec::new(),
        }
    }

    fn file_node(id: &str, location: &str) -> FileNode {
        FileNode {
            base: BaseNodeFields {
                id: id.to_string(),
                identity_key: String::new(),
                object_hash: None,
                name: location.to_string(),
                location: location.to_string(),
                language: "rust".to_string(),
                description: String::new(),
                parent_id: None,
                is_locked: false,
                lineage_locked: false,
                lock_owner: None,
                lock_reason: String::new(),
                task_ids: Vec::new(),
                structural_conflict: false,
            },
            extension: Some("rs".to_string()),
            source_blob_hash: None,
            imports: Vec::new(),
            exports: Vec::new(),
            leaf_children: Vec::new(),
        }
    }

    fn dir_node(id: &str, location: &str) -> DirNode {
        DirNode {
            base: BaseNodeFields {
                id: id.to_string(),
                identity_key: String::new(),
                object_hash: None,
                name: location.to_string(),
                location: location.to_string(),
                language: String::new(),
                description: String::new(),
                parent_id: None,
                is_locked: false,
                lineage_locked: false,
                lock_owner: None,
                lock_reason: String::new(),
                task_ids: Vec::new(),
                structural_conflict: false,
            },
            dir_children: Vec::new(),
            file_children: Vec::new(),
        }
    }

    #[test]
    fn identity_matcher_rule_1_exact_match() {
        let leaves = vec![leaf("src/foo.rs#bar", "bar", LeafKind::Function)];
        let matcher = IdentityMatcher::new(&leaves);
        assert_eq!(matcher.match_leaf("src/foo.rs", "bar", "function"), Some(0));
    }

    #[test]
    fn identity_matcher_rule_2_kind_shift() {
        let leaves = vec![leaf("src/foo.rs#Baz", "Baz", LeafKind::Struct)];
        let matcher = IdentityMatcher::new(&leaves);
        assert_eq!(matcher.match_leaf("src/foo.rs", "Baz", "enum"), Some(0));
    }

    #[test]
    fn identity_matcher_rejects_different_path() {
        let leaves = vec![leaf("src/foo.rs#bar", "bar", LeafKind::Function)];
        let matcher = IdentityMatcher::new(&leaves);
        assert_eq!(matcher.match_leaf("src/other.rs", "bar", "function"), None);
    }

    #[test]
    fn identity_matcher_rejects_different_name() {
        let leaves = vec![leaf("src/foo.rs#bar", "bar", LeafKind::Function)];
        let matcher = IdentityMatcher::new(&leaves);
        assert_eq!(matcher.match_leaf("src/foo.rs", "baz", "function"), None);
    }

    #[test]
    fn identity_matcher_prefers_rule_1_over_rule_2() {
        let leaves = vec![
            leaf("src/foo.rs#X", "X", LeafKind::Struct),
            leaf("src/foo.rs#X", "X", LeafKind::Trait),
        ];
        let matcher = IdentityMatcher::new(&leaves);
        assert_eq!(matcher.match_leaf("src/foo.rs", "X", "struct"), Some(0));
        assert_eq!(matcher.match_leaf("src/foo.rs", "X", "trait"), Some(1));
    }

    #[test]
    fn identity_matcher_rule_2_returns_first_candidate_deterministically() {
        let leaves = vec![
            leaf("src/foo.rs#X", "X", LeafKind::Struct),
            leaf("src/foo.rs#X", "X", LeafKind::Trait),
        ];
        let matcher = IdentityMatcher::new(&leaves);
        assert_eq!(matcher.match_leaf("src/foo.rs", "X", "enum"), Some(0));
    }

    #[test]
    fn split_leaf_location_splits_on_hash() {
        let (path, qn) = split_leaf_location("src/foo.rs#bar::baz");
        assert_eq!(path, "src/foo.rs");
        assert_eq!(qn, "bar::baz");
    }

    #[test]
    fn split_leaf_location_handles_missing_hash() {
        let (path, qn) = split_leaf_location("src/foo.rs");
        assert_eq!(path, "src/foo.rs");
        assert_eq!(qn, "");
    }

    #[test]
    fn dir_location_for_nested_file() {
        assert_eq!(dir_location_for_file("src/foo/bar.rs"), "src/foo/");
    }

    #[test]
    fn dir_location_for_root_file() {
        assert_eq!(dir_location_for_file("README.md"), "./");
    }

    #[test]
    fn append_task_ids_deduplicates() {
        let mut dest = vec!["T1".to_string(), "T2".to_string()];
        append_task_ids(&mut dest, &["T2".to_string(), "T3".to_string()]);
        assert_eq!(
            dest,
            vec!["T1".to_string(), "T2".to_string(), "T3".to_string()]
        );
    }

    #[test]
    fn leaf_kind_to_str_matches_extractor_lowercase() {
        assert_eq!(leaf_kind_to_str(&LeafKind::Function), "function");
        assert_eq!(leaf_kind_to_str(&LeafKind::Struct), "struct");
        assert_eq!(leaf_kind_to_str(&LeafKind::Trait), "trait");
    }

    #[test]
    fn apply_task_ids_to_nodes_mutates_only_touched() {
        let mut graph = CodebaseGraphV1 {
            root_dir_id: "dir:root".to_string(),
            dirs: vec![dir_node("dir:root", "./")],
            files: vec![
                file_node("file:a", "src/a.rs"),
                file_node("file:b", "src/b.rs"),
            ],
            leaves: Vec::new(),
        };
        let mut touched = HashSet::new();
        touched.insert("file:a".to_string());
        apply_task_ids_to_nodes(&mut graph, &touched, &["T20260421-0528".to_string()]);
        assert_eq!(graph.files[0].base.task_ids, vec!["T20260421-0528"]);
        assert!(graph.files[1].base.task_ids.is_empty());
        assert!(graph.dirs[0].base.task_ids.is_empty());
    }

    #[test]
    fn apply_task_ids_attributes_non_code_leaf_kinds_unchanged() {
        // Non-code leaves (Section / ConfigKey / Column added T20260422-1540)
        // must pick up task_ids via the same byte-range → leaf-id path as code
        // leaves. Task_ids are kind-agnostic because they live on
        // BaseNodeFields, not on LeafKind.
        let mut graph = CodebaseGraphV1 {
            root_dir_id: "dir:root".to_string(),
            dirs: vec![dir_node("dir:root", "./")],
            files: vec![file_node("file:a", "docs/README.md")],
            leaves: vec![
                leaf(
                    "docs/README.md#overview",
                    "Overview",
                    LeafKind::Section { depth: 1 },
                ),
                leaf("config.yaml#name", "name", LeafKind::ConfigKey),
                leaf("data.csv#id", "id", LeafKind::Column),
            ],
        };
        let mut touched = HashSet::new();
        for l in &graph.leaves {
            touched.insert(l.base.id.clone());
        }
        apply_task_ids_to_nodes(&mut graph, &touched, &["T20260422-1540".to_string()]);
        for l in &graph.leaves {
            assert_eq!(l.base.task_ids, vec!["T20260422-1540"]);
        }
    }

    #[test]
    fn apply_structural_conflict_marks_only_listed_nodes() {
        let mut graph = CodebaseGraphV1 {
            root_dir_id: "dir:root".to_string(),
            dirs: vec![dir_node("dir:root", "./")],
            files: vec![
                file_node("file:a", "src/a.rs"),
                file_node("file:b", "src/b.rs"),
            ],
            leaves: vec![leaf("src/a.rs#foo", "foo", LeafKind::Function)],
        };
        let mut ids = HashSet::new();
        ids.insert("file:a".to_string());
        ids.insert(graph.leaves[0].base.id.clone());
        apply_structural_conflict(&mut graph, &ids);
        assert!(graph.files[0].base.structural_conflict);
        assert!(!graph.files[1].base.structural_conflict);
        assert!(graph.leaves[0].base.structural_conflict);
        assert!(!graph.dirs[0].base.structural_conflict);
    }

    #[test]
    fn apply_structural_conflict_no_op_on_empty_ids() {
        let mut graph = CodebaseGraphV1 {
            root_dir_id: "dir:root".to_string(),
            dirs: vec![dir_node("dir:root", "./")],
            files: vec![file_node("file:a", "src/a.rs")],
            leaves: Vec::new(),
        };
        apply_structural_conflict(&mut graph, &HashSet::new());
        assert!(!graph.files[0].base.structural_conflict);
    }

    #[test]
    fn operation_log_dir_joins_orbit_root() {
        let p = operation_log_dir(Path::new("/repo/.orbit/knowledge"));
        assert_eq!(p, PathBuf::from("/repo/.orbit/operations"));
    }

    #[test]
    fn operation_log_dir_handles_rootless_path() {
        let p = operation_log_dir(Path::new("knowledge"));
        assert_eq!(p, PathBuf::from("operations"));
    }

    #[test]
    fn inspect_operation_log_reports_absent() {
        let dir = tempdir().unwrap();
        assert!(matches!(
            inspect_operation_log(dir.path(), "abc123"),
            OperationLogStatus::Absent
        ));
    }

    #[test]
    fn inspect_operation_log_reports_valid_for_empty_array() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("abc.json"), r#"{"operations":[]}"#).unwrap();
        assert!(matches!(
            inspect_operation_log(dir.path(), "abc"),
            OperationLogStatus::Valid
        ));
    }

    #[test]
    fn inspect_operation_log_reports_valid_for_missing_operations_field() {
        // Schema is permissive at this phase; missing `operations` deserializes
        // to an empty vec via `#[serde(default)]` and is still valid-but-noop.
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("abc.json"), r#"{}"#).unwrap();
        assert!(matches!(
            inspect_operation_log(dir.path(), "abc"),
            OperationLogStatus::Valid
        ));
    }

    #[test]
    fn inspect_operation_log_reports_malformed_for_bad_json() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("abc.json"), "{not valid json").unwrap();
        assert!(matches!(
            inspect_operation_log(dir.path(), "abc"),
            OperationLogStatus::Malformed(_)
        ));
    }

    #[test]
    fn inspect_operation_log_reports_malformed_for_wrong_type() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("abc.json"),
            r#"{"operations":"not an array"}"#,
        )
        .unwrap();
        assert!(matches!(
            inspect_operation_log(dir.path(), "abc"),
            OperationLogStatus::Malformed(_)
        ));
    }

    #[test]
    fn sort_and_dedup_task_ids_normalizes_across_node_types() {
        let mut graph = CodebaseGraphV1 {
            root_dir_id: "dir:root".to_string(),
            dirs: vec![dir_node("dir:root", "./")],
            files: vec![file_node("file:a", "src/a.rs")],
            leaves: vec![leaf("src/a.rs#foo", "foo", LeafKind::Function)],
        };
        graph.dirs[0].base.task_ids = vec!["T2".to_string(), "T1".to_string(), "T2".to_string()];
        graph.files[0].base.task_ids = vec!["Tb".to_string(), "Ta".to_string(), "Ta".to_string()];
        graph.leaves[0].base.task_ids = vec!["Ty".to_string(), "Tx".to_string(), "Ty".to_string()];
        sort_and_dedup_task_ids(&mut graph);
        assert_eq!(graph.dirs[0].base.task_ids, vec!["T1", "T2"]);
        assert_eq!(graph.files[0].base.task_ids, vec!["Ta", "Tb"]);
        assert_eq!(graph.leaves[0].base.task_ids, vec!["Tx", "Ty"]);
    }
}
