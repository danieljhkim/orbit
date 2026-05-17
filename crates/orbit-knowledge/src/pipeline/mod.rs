//! Build pipeline: scan → hash → build graph → persist.
//!
//! Each stage is a plain function operating on a shared [`PipelineContext`].

pub mod build;
pub mod context;
pub mod hash;
pub mod persist;
pub mod scan;

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant, UNIX_EPOCH};
use std::{fs, fs::OpenOptions, io::ErrorKind};

use crate::error::KnowledgeError;
use crate::graph::object_store::{
    CurrentRef, GraphObjectStore, resolve_graph_read_target, resolve_graph_write_target,
};
use crate::io::write_text_atomic_durable;
use context::{BuildConfig, PipelineContext};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const REFRESH_LOCK_NAME: &str = "refresh.lock";
const REFRESH_STATE_NAME: &str = "refresh_state.json";
const GRAPH_WAIT_TIMEOUT_MS: u64 = 2_500;
const GRAPH_WAIT_POLL_MS: u64 = 100;
const DEFAULT_DIRTY_REFRESH_DEBOUNCE_SECS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshStatus {
    Fresh,
    Rebuilt,
    SkippedDirtyDebounce,
    SkippedConcurrent,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RefreshState {
    #[serde(default)]
    last_refresh_at: Option<String>,
    #[serde(default)]
    dirty_fingerprint: Option<DirtyFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DirtyFingerprint {
    status_hash: String,
    path_count: usize,
    newest_mtime_ns: Option<u128>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitCheckoutIdentity {
    pub head_oid: String,
    pub tree_oid: String,
}

#[derive(Debug)]
enum RefreshPlan {
    Fresh,
    SkipDirtyDebounce,
    Rebuild {
        dirty_fingerprint: Option<DirtyFingerprint>,
        incremental: bool,
    },
}

struct RefreshLockGuard(std::fs::File);

impl Drop for RefreshLockGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Run the full build pipeline.
///
/// Scans the repo, computes hashes, builds the graph (dirs, files, leaves),
/// persists the graph to the content-addressed store, and writes the manifest.
pub fn run_build(config: BuildConfig) -> Result<PipelineContext, KnowledgeError> {
    let _lock = acquire_refresh_lock(&config.output_dir, false)?.ok_or_else(|| {
        KnowledgeError::io("failed to acquire refresh lock for run_build".to_string())
    })?;
    let dirty_fingerprint = git_dirty_fingerprint(&config.repo_path);
    let ctx = run_build_inner(config)?;
    save_refresh_state(&ctx.output_dir, dirty_fingerprint.as_ref())?;
    Ok(ctx)
}

fn run_build_inner(config: BuildConfig) -> Result<PipelineContext, KnowledgeError> {
    let build_target = resolve_graph_write_target(
        &config.repo_path,
        config.ref_name.as_ref().map(|ref_name| ref_name.as_str()),
    )
    .map_err(|error| {
        KnowledgeError::knowledge_unavailable(format!("resolve graph ref: {error}"))
    })?;
    let mut ctx = PipelineContext::new(config, build_target.requested, build_target.default);
    ctx.checkout_identity = git_checkout_identity(&ctx.repo_path);

    scan::scan_repo(&mut ctx)?;
    hash::compute_hashes(&mut ctx)?;
    hash::detect_changes(&mut ctx)?;

    build::build_graph_dirs(&mut ctx)?;
    build::build_graph_files(&mut ctx)?;
    build::build_graph_leaves(&mut ctx)?;

    persist::persist_graph(&ctx)?;
    persist::write_manifest(&ctx)?;
    hash::save_hash_cache(&ctx)?;

    Ok(ctx)
}

/// Ensure the knowledge graph is up-to-date with the current checkout.
///
/// Rebuilds when the checkout advances beyond the persisted manifest, when the
/// current branch does not have its own graph ref, or when a dirty worktree
/// fingerprint changes outside the debounce window. Dirty refreshes are
/// debounced and concurrent refreshes are single-flighted through `refresh.lock`
/// so read-side callers can reuse the current graph instead of stacking
/// rebuilds.
pub fn ensure_fresh(
    knowledge_dir: &Path,
    repo_path: &Path,
) -> Result<RefreshStatus, KnowledgeError> {
    let plan = compute_refresh_plan(knowledge_dir, repo_path)?;
    log_refresh_plan(repo_path, &plan);

    match plan {
        RefreshPlan::Fresh => return Ok(RefreshStatus::Fresh),
        RefreshPlan::SkipDirtyDebounce => return Ok(RefreshStatus::SkippedDirtyDebounce),
        RefreshPlan::Rebuild { .. } => {}
    }

    let Some(_lock) = acquire_refresh_lock(knowledge_dir, true)? else {
        wait_for_current_graph(knowledge_dir, repo_path);
        return Ok(RefreshStatus::SkippedConcurrent);
    };

    let plan = compute_refresh_plan(knowledge_dir, repo_path)?;
    let RefreshPlan::Rebuild {
        dirty_fingerprint,
        incremental,
    } = plan
    else {
        return Ok(match plan {
            RefreshPlan::Fresh => RefreshStatus::Fresh,
            RefreshPlan::SkipDirtyDebounce => RefreshStatus::SkippedDirtyDebounce,
            RefreshPlan::Rebuild { .. } => unreachable!(),
        });
    };

    let config = BuildConfig {
        repo_path: repo_path.to_path_buf(),
        output_dir: knowledge_dir.to_path_buf(),
        incremental,
        ref_name: None,
    };
    run_build_inner(config)
        .map_err(|e| KnowledgeError::knowledge_unavailable(format!("auto-refresh failed: {e}")))?;
    save_refresh_state(knowledge_dir, dirty_fingerprint.as_ref())
        .map_err(|e| KnowledgeError::knowledge_unavailable(format!("auto-refresh failed: {e}")))?;

    Ok(RefreshStatus::Rebuilt)
}

fn log_refresh_plan(repo_path: &Path, plan: &RefreshPlan) {
    let ref_name = resolve_graph_write_target(repo_path, None)
        .map(|target| target.requested.to_string())
        .unwrap_or_else(|_| "<unresolved>".to_string());
    tracing::info!(
        target: "orbit.knowledge.refresh",
        repo_path = %repo_path.display(),
        ref_name,
        refresh_plan = refresh_plan_variant(plan),
        "resolved knowledge graph refresh plan",
    );
}

fn refresh_plan_variant(plan: &RefreshPlan) -> &'static str {
    match plan {
        RefreshPlan::Fresh => "Fresh",
        RefreshPlan::SkipDirtyDebounce => "SkipDirtyDebounce",
        RefreshPlan::Rebuild { .. } => "Rebuild",
    }
}

fn git_checkout_identity(repo_path: &Path) -> Option<GitCheckoutIdentity> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD^{commit}", "HEAD^{tree}"])
        .current_dir(repo_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let head_oid = lines.next()?.to_string();
    let tree_oid = lines.next()?.to_string();
    Some(GitCheckoutIdentity { head_oid, tree_oid })
}

fn git_dirty_fingerprint(repo_path: &Path) -> Option<DirtyFingerprint> {
    let output = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .current_dir(repo_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    if output.stdout.is_empty() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut newest_mtime_ns = None;
    let mut path_count = 0usize;

    for line in stdout.lines() {
        for rel_path in candidate_paths_from_status_line(line) {
            path_count += 1;
            let abs_path = repo_path.join(&rel_path);
            let Ok(metadata) = fs::metadata(abs_path) else {
                continue;
            };
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            let Ok(duration) = modified.duration_since(UNIX_EPOCH) else {
                continue;
            };
            let mtime_ns = duration.as_nanos();
            newest_mtime_ns =
                Some(newest_mtime_ns.map_or(mtime_ns, |current: u128| current.max(mtime_ns)));
        }
    }

    Some(DirtyFingerprint {
        status_hash: format!("{:x}", Sha256::digest(stdout.as_bytes())),
        path_count,
        newest_mtime_ns,
    })
}

fn candidate_paths_from_status_line(line: &str) -> Vec<String> {
    let payload = line.get(3..).unwrap_or("").trim();
    if payload.is_empty() {
        return Vec::new();
    }
    payload
        .split(" -> ")
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .collect()
}

fn compute_refresh_plan(
    knowledge_dir: &Path,
    repo_path: &Path,
) -> Result<RefreshPlan, KnowledgeError> {
    let manifest_path = knowledge_dir.join("manifest.json");
    let current_ref = current_branch_ref(knowledge_dir, repo_path);
    let graph_available = current_ref.is_some();
    let dirty_fingerprint = git_dirty_fingerprint(repo_path);
    let checkout_identity = git_checkout_identity(repo_path);

    if let Some(dirty_fingerprint) = dirty_fingerprint {
        if manifest_path.is_file()
            && graph_available
            && refresh_state_within_cooldown(knowledge_dir, &dirty_fingerprint)?
        {
            return Ok(RefreshPlan::SkipDirtyDebounce);
        }
        return Ok(RefreshPlan::Rebuild {
            dirty_fingerprint: Some(dirty_fingerprint),
            incremental: manifest_path.is_file(),
        });
    }

    if manifest_path.is_file() {
        let raw = fs::read_to_string(&manifest_path)
            .map_err(|e| KnowledgeError::knowledge_unavailable(format!("read manifest: {e}")))?;
        let _: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| KnowledgeError::knowledge_unavailable(format!("parse manifest: {e}")))?;
        if !graph_available {
            return Ok(RefreshPlan::Rebuild {
                dirty_fingerprint: None,
                incremental: true,
            });
        }
        let Some(checkout_identity) = checkout_identity else {
            return Ok(RefreshPlan::Rebuild {
                dirty_fingerprint: None,
                incremental: true,
            });
        };
        if stored_checkout_identity_matches(current_ref.as_ref(), &checkout_identity) {
            return Ok(RefreshPlan::Fresh);
        }
        return Ok(RefreshPlan::Rebuild {
            dirty_fingerprint: None,
            incremental: true,
        });
    }

    Ok(RefreshPlan::Rebuild {
        dirty_fingerprint: None,
        incremental: false,
    })
}

fn stored_checkout_identity_matches(
    current_ref: Option<&CurrentRef>,
    checkout_identity: &GitCheckoutIdentity,
) -> bool {
    if let Some(current_ref) = current_ref {
        if let Some(stored_head_oid) = current_ref.git_head_oid.as_deref() {
            return stored_head_oid == checkout_identity.head_oid;
        }
        if let Some(stored_tree_oid) = current_ref.git_tree_oid.as_deref() {
            return stored_tree_oid == checkout_identity.tree_oid;
        }
    }

    false
}

fn refresh_state_within_cooldown(
    knowledge_dir: &Path,
    dirty_fingerprint: &DirtyFingerprint,
) -> Result<bool, KnowledgeError> {
    let state = load_refresh_state(knowledge_dir)?;
    let Some(last_refresh_at) = state
        .last_refresh_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
    else {
        return Ok(false);
    };
    let Some(last_dirty_fingerprint) = state.dirty_fingerprint.as_ref() else {
        return Ok(false);
    };
    if last_dirty_fingerprint != dirty_fingerprint {
        return Ok(false);
    }

    let elapsed =
        chrono::Utc::now().signed_duration_since(last_refresh_at.with_timezone(&chrono::Utc));
    Ok(elapsed.to_std().unwrap_or_default() < dirty_refresh_cooldown())
}

fn load_refresh_state(knowledge_dir: &Path) -> Result<RefreshState, KnowledgeError> {
    let state_path = knowledge_dir.join(REFRESH_STATE_NAME);
    if !state_path.is_file() {
        return Ok(RefreshState::default());
    }

    let raw = fs::read_to_string(&state_path)
        .map_err(|error| KnowledgeError::io(format!("read refresh state: {error}")))?;
    serde_json::from_str(&raw)
        .map_err(|error| KnowledgeError::invalid_data(format!("parse refresh state: {error}")))
}

fn save_refresh_state(
    knowledge_dir: &Path,
    dirty_fingerprint: Option<&DirtyFingerprint>,
) -> Result<(), KnowledgeError> {
    let state = RefreshState {
        last_refresh_at: Some(chrono::Utc::now().to_rfc3339()),
        dirty_fingerprint: dirty_fingerprint.cloned(),
    };
    let json = serde_json::to_string_pretty(&state).map_err(|error| {
        KnowledgeError::invalid_data(format!("serialize refresh state: {error}"))
    })?;
    write_text_atomic_durable(
        &knowledge_dir.join(REFRESH_STATE_NAME),
        &format!("{json}\n"),
    )
    .map_err(|error| KnowledgeError::io(format!("write refresh state: {error}")))
}

fn acquire_refresh_lock(
    knowledge_dir: &Path,
    nonblocking: bool,
) -> Result<Option<RefreshLockGuard>, KnowledgeError> {
    fs::create_dir_all(knowledge_dir)
        .map_err(|error| KnowledgeError::io(format!("mkdir knowledge dir: {error}")))?;
    let lock_path = knowledge_dir.join(REFRESH_LOCK_NAME);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| KnowledgeError::io(format!("open refresh lock: {error}")))?;

    if nonblocking {
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(RefreshLockGuard(file))),
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(KnowledgeError::io(format!("lock refresh lock: {error}"))),
        }
    } else {
        file.lock_exclusive()
            .map_err(|error| KnowledgeError::io(format!("lock refresh lock: {error}")))?;
        Ok(Some(RefreshLockGuard(file)))
    }
}

fn current_branch_graph_available(knowledge_dir: &Path, repo_path: &Path) -> bool {
    current_branch_ref(knowledge_dir, repo_path).is_some()
}

fn current_branch_ref(knowledge_dir: &Path, repo_path: &Path) -> Option<CurrentRef> {
    if !knowledge_dir.join("manifest.json").is_file() {
        return None;
    }

    let Ok(read_target) = resolve_graph_read_target(Some(repo_path), None) else {
        return None;
    };

    let store = GraphObjectStore::new(knowledge_dir.join("graph"));
    if store
        .prepare_refs_layout(read_target.default.as_ref())
        .is_err()
    {
        return None;
    }

    if !store.ref_path(&read_target.requested).is_file() {
        return None;
    }

    store.read_ref(&read_target.requested).ok()
}

fn wait_for_current_graph(knowledge_dir: &Path, repo_path: &Path) {
    let deadline = Instant::now() + Duration::from_millis(GRAPH_WAIT_TIMEOUT_MS);
    while Instant::now() < deadline {
        if current_branch_graph_available(knowledge_dir, repo_path) {
            return;
        }
        std::thread::sleep(Duration::from_millis(GRAPH_WAIT_POLL_MS));
    }
}

fn dirty_refresh_cooldown() -> Duration {
    let seconds = std::env::var("ORBIT_KNOWLEDGE_REFRESH_DEBOUNCE_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_DIRTY_REFRESH_DEBOUNCE_SECS);
    Duration::from_secs(seconds)
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    use tempfile::tempdir;
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Level, Metadata, Subscriber};

    use super::*;

    #[test]
    fn ensure_fresh_logs_repo_ref_and_refresh_plan() {
        let fixture = GitRepoFixture::new();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let subscriber = CaptureSubscriber {
            captured: Arc::clone(&captured),
        };

        tracing::subscriber::with_default(subscriber, || {
            ensure_fresh(&fixture.knowledge_dir, &fixture.repo).expect("ensure fresh");
        });

        let logs = captured.lock().expect("lock captured logs").join("\n");
        assert!(logs.contains("orbit.knowledge.refresh"));
        assert!(logs.contains(&format!("repo_path={}", fixture.repo.display())));
        assert!(logs.contains("ref_name"));
        assert!(logs.contains("agent-main"));
        assert!(logs.contains("refresh_plan"));
        assert!(logs.contains("Rebuild"));
    }

    struct CaptureSubscriber {
        captured: Arc<Mutex<Vec<String>>>,
    }

    impl Subscriber for CaptureSubscriber {
        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            metadata.target() == "orbit.knowledge.refresh" && *metadata.level() <= Level::INFO
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            if !self.enabled(event.metadata()) {
                return;
            }
            let mut visitor = FieldCapture::default();
            event.record(&mut visitor);
            let fields = visitor.fields.join(" ");
            self.captured
                .lock()
                .expect("lock captured events")
                .push(format!("{} {fields}", event.metadata().target()));
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    #[derive(Default)]
    struct FieldCapture {
        fields: Vec<String>,
    }

    impl Visit for FieldCapture {
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.fields.push(format!("{}={value:?}", field.name()));
        }
    }

    struct GitRepoFixture {
        _root: tempfile::TempDir,
        repo: PathBuf,
        knowledge_dir: PathBuf,
    }

    impl GitRepoFixture {
        fn new() -> Self {
            let root = tempdir().expect("create tempdir");
            let repo = root.path().join("repo");
            let knowledge_dir = root.path().join("knowledge");
            std::fs::create_dir_all(repo.join("src")).expect("create src dir");
            run_git(root.path(), &["init", repo.to_str().expect("repo path")]);
            run_git(&repo, &["config", "user.email", "test@example.com"]);
            run_git(&repo, &["config", "user.name", "Test User"]);
            std::fs::write(
                repo.join("Cargo.toml"),
                "[package]\nname = \"refresh_log_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n",
            )
            .expect("write manifest");
            std::fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").expect("write lib");
            run_git(&repo, &["add", "Cargo.toml", "src/lib.rs"]);
            run_git(&repo, &["commit", "-m", "initial"]);
            run_git(&repo, &["branch", "-M", "agent-main"]);

            Self {
                _root: root,
                repo,
                knowledge_dir,
            }
        }
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
