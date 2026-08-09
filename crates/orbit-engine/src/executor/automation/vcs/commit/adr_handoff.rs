//! Host-side staging handoff for proposed ADR bundles allocated during a run
//! (ADR-0338).
//!
//! A proposed ADR is written to the *run worktree's* own `.orbit/adrs/proposed/`
//! partition, and the workspace-init gitignore template keeps that partition
//! ignored — proposed drafts are local-only until publication. `git add --all`
//! honours that ignore, so a draft documenting the code in the very same change
//! would silently never ship.
//!
//! The implementing agent cannot close the gap itself: in a linked worktree
//! `.git` is a file pointing at the main checkout's worktree metadata, which
//! sits outside the implementer's filesystem grant and is bound read-only, so
//! creating `index.lock` fails there. The commit step runs unsandboxed from the
//! engine, which is the one place that *can* stage the bundle — hence this
//! handoff.
//!
//! Two rules shape it. It is scoped to `proposed/`, so accepted and superseded
//! partitions keep whatever behaviour their own ignore rules give them. And it
//! fails closed: when the bundle cannot be staged, delivery refuses with a
//! diagnostic naming the bundle and the supported path, because a dropped draft
//! and a fabricated ADR id are both worse than a refused commit.

use std::collections::BTreeSet;
use std::path::Path;

use orbit_common::types::OrbitError;
use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};

use super::super::git::{git_output_paths, git_success};

/// Repo-relative root of the local-until-publication ADR partition.
const PROPOSED_ADR_DIR: &str = ".orbit/adrs/proposed";

/// The two files that make up one ADR bundle on disk. Anything else under the
/// partition (lock files, index scratch) is deliberately not delivered.
const BUNDLE_FILE_NAMES: [&str; 2] = ["adr.yaml", "body.md"];

/// Force-stage every proposed ADR bundle the run allocated but git is ignoring.
///
/// Returns the repo-relative paths that were handed off, which is empty in the
/// common case: a workspace that tracks its proposed partition needs no handoff
/// because `git add --all` already covers it.
///
/// Runs *before* `git add --all` so that a checkout whose git metadata is
/// read-only refuses with the bundle-naming diagnostic below rather than with a
/// bare `git add` failure that names nothing an operator can act on.
pub(super) fn stage_proposed_adr_bundles(
    workspace_path: &Path,
    task_id: &str,
) -> Result<Vec<String>, OrbitError> {
    let candidates = bundle_files_on_disk(workspace_path);
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let ignored = ignored_paths(workspace_path, &candidates)?;
    if ignored.is_empty() {
        return Ok(Vec::new());
    }

    let mut args = vec!["add".to_string(), "--force".to_string(), "--".to_string()];
    args.extend(ignored.iter().cloned());
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    git_success(workspace_path, &arg_refs)
        .map_err(|error| refusal(task_id, workspace_path, &ignored, &error.to_string()))?;

    // Staging reported success; confirm the index actually holds every bundle
    // file before letting the commit proceed. A path that silently missed the
    // index would ship the code without the decision that documents it.
    let indexed = indexed_paths(workspace_path, &ignored)?;
    let missing = ignored
        .iter()
        .filter(|path| !indexed.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(refusal(
            task_id,
            workspace_path,
            &missing,
            "`git add --force` reported success but the paths are absent from the index",
        ));
    }

    Ok(ignored)
}

/// Every `adr.yaml`/`body.md` under the proposed partition, repo-relative with
/// forward slashes. A missing partition is the normal case, not an error.
fn bundle_files_on_disk(workspace_path: &Path) -> Vec<String> {
    let root = workspace_path.join(PROPOSED_ADR_DIR);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };

    let mut found = BTreeSet::new();
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let Some(bundle) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        for name in BUNDLE_FILE_NAMES {
            if entry.path().join(name).is_file() {
                found.insert(format!("{PROPOSED_ADR_DIR}/{bundle}/{name}"));
            }
        }
    }
    found.into_iter().collect()
}

/// Ask git which of `candidates` its ignore rules exclude.
///
/// `check-ignore` is used rather than `ls-files --others --ignored` because it
/// answers from the ignore rules alone without refreshing or locking the index,
/// so discovery still works in the read-only-metadata case this handoff exists
/// to diagnose. Exit code 1 is its documented "nothing matched" answer, not a
/// failure.
fn ignored_paths(workspace_path: &Path, candidates: &[String]) -> Result<Vec<String>, OrbitError> {
    // NUL-delimited on stdin, so a path is never split on whitespace and the
    // argument list cannot overflow for a run that allocated many drafts.
    let mut stdin = candidates.join("\0").into_bytes();
    stdin.push(0);
    let result = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: vec![
                "check-ignore".to_string(),
                "-z".to_string(),
                "--stdin".to_string(),
            ],
            current_dir: Some(workspace_path.to_string_lossy().to_string()),
            timeout_ms: Some(30_000),
            stdin_mode: StdinMode::Bytes(stdin),
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    match result.exit_code {
        Some(0) => Ok(result
            .stdout
            .split('\0')
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect()),
        Some(1) => Ok(Vec::new()),
        _ => Err(OrbitError::Execution(format!(
            "git check-ignore failed in '{}': {}",
            workspace_path.display(),
            result.stderr.trim()
        ))),
    }
}

/// The subset of `paths` present in the index, whether newly staged or already
/// tracked.
fn indexed_paths(workspace_path: &Path, paths: &[String]) -> Result<BTreeSet<String>, OrbitError> {
    let mut args = vec!["ls-files".to_string(), "-z".to_string(), "--".to_string()];
    args.extend(paths.iter().cloned());
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    Ok(git_output_paths(workspace_path, &arg_refs)?
        .into_iter()
        .collect())
}

/// Refuse delivery, naming the bundle that would have been dropped and the
/// supported way to get it in.
fn refusal(task_id: &str, workspace_path: &Path, bundle: &[String], cause: &str) -> OrbitError {
    OrbitError::Execution(format!(
        "commit_batch_changes: refusing to deliver task '{task_id}' from worktree '{}' without \
         its proposed ADR bundle. These files are ignored by the workspace's own gitignore \
         policy and could not be force-staged: {}. Cause: {cause}. Supported path: stage the \
         bundle host-side from an unsandboxed checkout (`git add --force -- <paths>`) and re-run \
         delivery, or publish the ADR so it leaves the local-only partition. Orbit did not drop \
         the bundle, invent an ADR id, or commit the code without it",
        workspace_path.display(),
        bundle.join(", ")
    ))
}
