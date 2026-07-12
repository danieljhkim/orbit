//! Shared, domain-neutral garbage-collection planning and apply framework.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use orbit_common::types::{OrbitError, audit_execution_id};
use serde::{Deserialize, Serialize};

const LOCK_WAIT: Duration = Duration::from_secs(2);
const LOCK_POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GcTarget {
    Worktrees,
    Runs,
    Logs,
    Diagnostics,
    Audit,
    Skills,
    Tasks,
    All,
}

impl std::fmt::Display for GcTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = serde_json::to_value(self).map_err(|_| std::fmt::Error)?;
        formatter.write_str(value.as_str().ok_or(std::fmt::Error)?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GcScope {
    Global {
        root: PathBuf,
    },
    Workspace {
        workspace_id: Option<String>,
        root: PathBuf,
    },
}

impl GcScope {
    pub fn root(&self) -> &Path {
        match self {
            Self::Global { root } | Self::Workspace { root, .. } => root,
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Global { .. } => "global".to_string(),
            Self::Workspace { workspace_id, .. } => workspace_id
                .as_deref()
                .map_or_else(|| "workspace".to_string(), |id| format!("workspace:{id}")),
        }
    }
}

pub trait GcClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default)]
pub struct SystemGcClock;

impl GcClock for SystemGcClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub struct GcContext<'a> {
    pub scope: &'a GcScope,
    pub retention_override: Option<&'a str>,
    pub clock: &'a dyn GcClock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcCandidate {
    pub id: String,
    pub action: String,
    pub path: Option<PathBuf>,
    pub bytes: Option<u64>,
    pub ownership_evidence: String,
    pub retention_evidence: String,
    pub expected_state: String,
    #[serde(default)]
    pub allow_owned_symlink: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcSkip {
    pub id: String,
    pub code: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcItemError {
    pub id: String,
    pub phase: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcPlan {
    pub target: GcTarget,
    pub config_source: String,
    pub scanned: u64,
    pub scanned_bytes: Option<u64>,
    pub candidates: Vec<GcCandidate>,
    pub skipped: Vec<GcSkip>,
    pub errors: Vec<GcItemError>,
}

impl GcPlan {
    pub fn empty(target: GcTarget) -> Self {
        Self {
            target,
            config_source: "builtin".to_string(),
            scanned: 0,
            scanned_bytes: Some(0),
            candidates: Vec::new(),
            skipped: Vec::new(),
            errors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcRevalidation {
    Ready,
    Skip { code: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcMutation {
    pub reclaimed_bytes: Option<u64>,
}

pub trait GcCollector {
    fn target(&self) -> GcTarget;
    fn plan(&self, context: &GcContext<'_>) -> Result<GcPlan, OrbitError>;
    fn revalidate(
        &self,
        candidate: &GcCandidate,
        context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError>;
    fn apply(
        &self,
        candidate: &GcCandidate,
        context: &GcContext<'_>,
    ) -> Result<GcMutation, OrbitError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GcMode {
    Plan,
    Apply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GcOutcome {
    Clean,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcCounts {
    pub scanned: u64,
    pub eligible: u64,
    pub reclaimed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcBytes {
    pub scanned: u64,
    pub eligible: u64,
    pub reclaimed: u64,
    pub estimate_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GcItemStatus {
    Eligible,
    Reclaimed,
    Skipped,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcItemReport {
    pub id: String,
    pub action: String,
    pub status: GcItemStatus,
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcTargetReport {
    pub target: GcTarget,
    pub counts: GcCounts,
    pub bytes: GcBytes,
    pub items: Vec<GcItemReport>,
    pub skipped: Vec<GcSkip>,
    pub errors: Vec<GcItemError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcReport {
    pub schema_version: u32,
    pub mode: GcMode,
    pub plan_id: String,
    pub scope: GcScope,
    pub config_source: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub outcome: GcOutcome,
    pub manifest_path: Option<PathBuf>,
    pub targets: Vec<GcTargetReport>,
}

impl GcReport {
    pub fn has_errors(&self) -> bool {
        self.targets.iter().any(|target| !target.errors.is_empty())
    }
}

pub struct GcRequest<'a> {
    pub apply: bool,
    pub scope: GcScope,
    pub retention_override: Option<&'a str>,
    pub global_state_dir: &'a Path,
    pub clock: &'a dyn GcClock,
}

pub fn execute_gc(
    collector: &dyn GcCollector,
    request: GcRequest<'_>,
) -> Result<GcReport, OrbitError> {
    let started_at = request.clock.now();
    let plan_id = audit_execution_id("gc");
    let context = GcContext {
        scope: &request.scope,
        retention_override: request.retention_override,
        clock: request.clock,
    };

    // ADR-0220: apply holds the host lock while creating and consuming one frozen plan.
    let _lock = if request.apply {
        Some(acquire_gc_lock(
            request.global_state_dir,
            &plan_id,
            collector.target(),
            &request.scope,
            started_at,
        )?)
    } else {
        None
    };
    let plan = collector.plan(&context)?;
    if plan.target != collector.target() {
        return Err(OrbitError::Execution(format!(
            "GC collector reported target `{}` while registered as `{}`",
            plan.target,
            collector.target()
        )));
    }

    let eligible_bytes = plan
        .candidates
        .iter()
        .filter_map(|candidate| candidate.bytes)
        .fold(0_u64, u64::saturating_add);
    let estimate_complete = plan.scanned_bytes.is_some()
        && plan
            .candidates
            .iter()
            .all(|candidate| candidate.bytes.is_some());
    let mut report = GcTargetReport {
        target: plan.target,
        counts: GcCounts {
            scanned: plan.scanned,
            eligible: plan.candidates.len() as u64,
            reclaimed: 0,
        },
        bytes: GcBytes {
            scanned: plan.scanned_bytes.unwrap_or_default(),
            eligible: eligible_bytes,
            reclaimed: 0,
            estimate_complete,
        },
        items: plan
            .candidates
            .iter()
            .map(|candidate| GcItemReport {
                id: candidate.id.clone(),
                action: candidate.action.clone(),
                status: GcItemStatus::Eligible,
                bytes: candidate.bytes,
            })
            .collect(),
        skipped: plan.skipped,
        errors: plan.errors,
    };

    let manifest_path = request.apply.then(|| {
        request
            .global_state_dir
            .join("gc")
            .join("manifests")
            .join(format!("{plan_id}.jsonl"))
    });
    if request.apply {
        for (candidate, item) in plan.candidates.iter().zip(report.items.iter_mut()) {
            let result = apply_candidate(
                collector,
                &context,
                candidate,
                manifest_path.as_deref().ok_or_else(|| {
                    OrbitError::Execution("GC apply manifest path was not created".to_string())
                })?,
                &plan_id,
            );
            match result {
                Ok(ApplyResult::Reclaimed {
                    bytes,
                    manifest_error,
                }) => {
                    item.status = GcItemStatus::Reclaimed;
                    item.bytes = bytes.or(item.bytes);
                    report.counts.reclaimed = report.counts.reclaimed.saturating_add(1);
                    report.bytes.reclaimed = report
                        .bytes
                        .reclaimed
                        .saturating_add(bytes.or(candidate.bytes).unwrap_or_default());
                    if bytes.is_none() && candidate.bytes.is_none() {
                        report.bytes.estimate_complete = false;
                    }
                    if let Some(error) = manifest_error {
                        report.errors.push(error);
                    }
                }
                Ok(ApplyResult::Skipped(skip)) => {
                    item.status = GcItemStatus::Skipped;
                    report.skipped.push(skip);
                }
                Err(error) => {
                    item.status = GcItemStatus::Error;
                    report.errors.push(error);
                }
            }
        }
    }

    let outcome = if report.errors.is_empty() {
        GcOutcome::Clean
    } else if report.counts.reclaimed > 0 {
        GcOutcome::Partial
    } else {
        GcOutcome::Failed
    };
    Ok(GcReport {
        schema_version: 1,
        mode: if request.apply {
            GcMode::Apply
        } else {
            GcMode::Plan
        },
        plan_id,
        scope: request.scope,
        config_source: plan.config_source,
        started_at,
        finished_at: request.clock.now(),
        outcome,
        manifest_path: manifest_path.filter(|path| path.exists()),
        targets: vec![report],
    })
}

enum ApplyResult {
    Reclaimed {
        bytes: Option<u64>,
        manifest_error: Option<GcItemError>,
    },
    Skipped(GcSkip),
}

fn apply_candidate(
    collector: &dyn GcCollector,
    context: &GcContext<'_>,
    candidate: &GcCandidate,
    manifest_path: &Path,
    plan_id: &str,
) -> Result<ApplyResult, GcItemError> {
    match collector.revalidate(candidate, context) {
        Ok(GcRevalidation::Ready) => {}
        Ok(GcRevalidation::Skip { code, reason }) => {
            return Ok(ApplyResult::Skipped(GcSkip {
                id: candidate.id.clone(),
                code,
                reason,
            }));
        }
        Err(error) => return Err(item_error(candidate, "revalidate", error)),
    }
    if let Some(path) = &candidate.path
        && let Err(error) =
            validate_candidate_path(context.scope.root(), path, candidate.allow_owned_symlink)
    {
        return Err(item_error(candidate, "revalidate", error));
    }
    // Persist intent before mutation so even interruption between mutation and
    // result recording cannot leave an unaudited destructive attempt.
    append_manifest(
        manifest_path,
        plan_id,
        collector.target(),
        candidate,
        None,
        "attempting",
    )
    .map_err(|error| item_error(candidate, "manifest", error))?;
    let mutation = collector
        .apply(candidate, context)
        .map_err(|error| item_error(candidate, "apply", error))?;
    let manifest_error = append_manifest(
        manifest_path,
        plan_id,
        collector.target(),
        candidate,
        mutation.reclaimed_bytes,
        "reclaimed",
    )
    .err()
    .map(|error| item_error(candidate, "manifest", error));
    Ok(ApplyResult::Reclaimed {
        bytes: mutation.reclaimed_bytes,
        manifest_error,
    })
}

fn item_error(candidate: &GcCandidate, phase: &str, error: OrbitError) -> GcItemError {
    GcItemError {
        id: candidate.id.clone(),
        phase: phase.to_string(),
        code: "operation_failed".to_string(),
        message: error.to_string(),
    }
}

#[derive(Serialize)]
struct ManifestEntry<'a> {
    schema_version: u32,
    plan_id: &'a str,
    target: GcTarget,
    candidate_id: &'a str,
    ownership_evidence: &'a str,
    retention_evidence: &'a str,
    action: &'a str,
    reclaimed_bytes: Option<u64>,
    result: &'static str,
}

fn append_manifest(
    path: &Path,
    plan_id: &str,
    target: GcTarget,
    candidate: &GcCandidate,
    reclaimed_bytes: Option<u64>,
    result: &'static str,
) -> Result<(), OrbitError> {
    let parent = path.parent().ok_or_else(|| {
        OrbitError::Execution("GC manifest path has no parent directory".to_string())
    })?;
    fs::create_dir_all(parent)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(
        &mut file,
        &ManifestEntry {
            schema_version: 1,
            plan_id,
            target,
            candidate_id: &candidate.id,
            ownership_evidence: &candidate.ownership_evidence,
            retention_evidence: &candidate.retention_evidence,
            action: &candidate.action,
            reclaimed_bytes,
            result,
        },
    )
    .map_err(|error| OrbitError::Io(error.to_string()))?;
    file.write_all(b"\n")?;
    file.sync_data()?;
    Ok(())
}

struct GcLockGuard {
    file: File,
}

impl Drop for GcLockGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn acquire_gc_lock(
    global_state_dir: &Path,
    plan_id: &str,
    target: GcTarget,
    scope: &GcScope,
    acquired_at: DateTime<Utc>,
) -> Result<GcLockGuard, OrbitError> {
    fs::create_dir_all(global_state_dir)?;
    let path = global_state_dir.join("gc.lock");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)?;
    let started = Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => break,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if started.elapsed() >= LOCK_WAIT {
                    return Err(OrbitError::Execution(format!(
                        "timed out waiting for host-global GC lock `{}`",
                        path.display()
                    )));
                }
                thread::sleep(LOCK_POLL);
            }
            Err(error) => return Err(error.into()),
        }
    }
    file.set_len(0)?;
    let metadata = serde_json::json!({
        "pid": std::process::id(),
        "process_start_identity": process_start_identity(),
        "plan_id": plan_id,
        "command": format!("gc {target}"),
        "scope": scope.label(),
        "acquired_at": acquired_at,
    });
    serde_json::to_writer(&mut file, &metadata)
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    file.sync_data()?;
    Ok(GcLockGuard { file })
}

#[cfg(target_os = "linux")]
fn process_start_identity() -> Option<String> {
    let stat = fs::read_to_string("/proc/self/stat").ok()?;
    let (_, fields) = stat.rsplit_once(") ")?;
    // `/proc/<pid>/stat` field 22 is the process start time in clock ticks.
    fields.split_whitespace().nth(19).map(ToString::to_string)
}

#[cfg(not(target_os = "linux"))]
fn process_start_identity() -> Option<String> {
    None
}

pub fn validate_candidate_path(
    owned_root: &Path,
    candidate: &Path,
    allow_final_symlink: bool,
) -> Result<(), OrbitError> {
    // ADR-0220: containment and no-follow symlink checks are non-bypassable mutation gates.
    if candidate
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(OrbitError::InvalidInput(format!(
            "GC candidate contains parent traversal: {}",
            candidate.display()
        )));
    }
    let root = owned_root.canonicalize().map_err(|error| {
        OrbitError::Io(format!(
            "cannot resolve GC owned root `{}`: {error}",
            owned_root.display()
        ))
    })?;
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    if !absolute.starts_with(&root) || absolute == root {
        return Err(OrbitError::PolicyDenied(format!(
            "GC candidate escapes or equals owned root: {}",
            candidate.display()
        )));
    }
    let relative = absolute.strip_prefix(&root).map_err(|_| {
        OrbitError::PolicyDenied(format!(
            "GC candidate escapes owned root: {}",
            candidate.display()
        ))
    })?;
    let mut current = root;
    let component_count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        if !matches!(component, Component::Normal(_)) {
            return Err(OrbitError::InvalidInput(format!(
                "GC candidate has an unsupported path component: {}",
                candidate.display()
            )));
        }
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink()
            && !(allow_final_symlink && index + 1 == component_count)
        {
            return Err(OrbitError::PolicyDenied(format!(
                "GC candidate crosses a symlink: {}",
                current.display()
            )));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct EmptyGcCollector {
    target: GcTarget,
}

impl EmptyGcCollector {
    pub fn new(target: GcTarget) -> Self {
        Self { target }
    }
}

impl GcCollector for EmptyGcCollector {
    fn target(&self) -> GcTarget {
        self.target
    }

    fn plan(&self, _context: &GcContext<'_>) -> Result<GcPlan, OrbitError> {
        let mut plan = GcPlan::empty(self.target);
        plan.skipped.push(GcSkip {
            id: self.target.to_string(),
            code: "collector_not_implemented".to_string(),
            reason: "target grammar is registered; domain collection lands in a dependent task"
                .to_string(),
        });
        Ok(plan)
    }

    fn revalidate(
        &self,
        _candidate: &GcCandidate,
        _context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError> {
        Ok(GcRevalidation::Ready)
    }

    fn apply(
        &self,
        _candidate: &GcCandidate,
        _context: &GcContext<'_>,
    ) -> Result<GcMutation, OrbitError> {
        Err(OrbitError::Execution(
            "empty GC collector cannot mutate candidates".to_string(),
        ))
    }
}
