//! Unified retention collector for legacy and v2 audit evidence.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::OrbitError;
use orbit_common::utility::audit_writer_guard;
use orbit_store::{AuditGcLegacyRow, Store, V2AuditEventRow};
use serde_json::Value;

use super::gc::{
    GcCandidate, GcCollector, GcContext, GcItemError, GcMutation, GcPlan, GcRevalidation, GcSkip,
    GcTarget,
};

const DEFAULT_RETENTION: &str = "90d";

/// Collects all audit surfaces owned by one workspace. Database candidates
/// are removed before JSONL envelopes, and blobs are always last. Blob
/// revalidation repeats the mark phase after those mutations, so interruption
/// and restart cannot leave a retained envelope pointing at a swept blob.
pub struct AuditGcCollector {
    store: Store,
    workspace_id: String,
    audit_root: PathBuf,
}

impl AuditGcCollector {
    pub fn new(store: Store, workspace_id: impl Into<String>, orbit_root: &Path) -> Self {
        Self {
            store,
            workspace_id: workspace_id.into(),
            audit_root: orbit_root.join("state").join("audit"),
        }
    }

    fn cutoff(&self, context: &GcContext<'_>) -> Result<DateTime<Utc>, OrbitError> {
        let retention = context.retention_override.unwrap_or(DEFAULT_RETENTION);
        let seconds = parse_duration_seconds(retention)?;
        context
            .clock
            .now()
            .checked_sub_signed(Duration::seconds(seconds))
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!("retention `{retention}` is too large"))
            })
    }

    fn snapshot(&self, context: &GcContext<'_>) -> Result<AuditSnapshot, OrbitError> {
        let cutoff = self.cutoff(context)?;
        let orbit_root = context.scope.root();
        let repo_root = orbit_root.parent().unwrap_or(orbit_root);
        let legacy = self.store.list_legacy_audit_rows_for_gc()?;
        let v2 = self.store.list_v2_audit_events_for_gc(&self.workspace_id)?;
        let protected = protected_evidence(orbit_root)?;
        let jsonl = scan_jsonl(&self.audit_root.join("v2_loop"), cutoff, &protected.run_ids)?;

        let mut retained_refs = protected.blob_refs;
        for row in legacy.iter().filter(|row| {
            row.timestamp >= cutoff
                || !working_directory_in_scope(&row.working_directory, repo_root)
        }) {
            collect_legacy_refs(row, &mut retained_refs);
        }
        for row in v2
            .iter()
            .filter(|row| row.ts >= cutoff || protected.run_ids.contains(&row.run_id))
        {
            collect_blob_refs(&row.payload_json, &mut retained_refs);
        }
        for file in &jsonl {
            if !file.eligible {
                retained_refs.extend(file.blob_refs.iter().cloned());
            }
        }
        Ok(AuditSnapshot {
            cutoff,
            repo_root: repo_root.to_path_buf(),
            legacy,
            v2,
            jsonl,
            retained_refs,
            protected_run_ids: protected.run_ids,
        })
    }

    fn blob_is_referenced(&self, hash: &str, context: &GcContext<'_>) -> Result<bool, OrbitError> {
        Ok(self.snapshot(context)?.retained_refs.contains(hash))
    }
}

impl GcCollector for AuditGcCollector {
    fn target(&self) -> GcTarget {
        GcTarget::Audit
    }

    fn plan(&self, context: &GcContext<'_>) -> Result<GcPlan, OrbitError> {
        let snapshot = self.snapshot(context)?;
        let mut plan = GcPlan::empty(GcTarget::Audit);
        plan.config_source = context
            .retention_override
            .map_or_else(|| "builtin:90d".to_string(), |value| format!("cli:{value}"));

        for row in &snapshot.legacy {
            plan.scanned += 1;
            if !working_directory_in_scope(&row.working_directory, &snapshot.repo_root) {
                plan.skipped.push(skip(
                    format!("legacy:{}", row.id),
                    "other_workspace",
                    "legacy event working directory is outside this workspace",
                ));
            } else if row.timestamp < snapshot.cutoff {
                plan.candidates.push(candidate(
                    format!("legacy:{}", row.id),
                    "delete_legacy_event",
                    None,
                    format!("legacy audit row {} belongs to workspace path", row.id),
                    format!("timestamp {} is before {}", row.timestamp, snapshot.cutoff),
                    row.timestamp.to_rfc3339(),
                ));
            }
        }
        for row in &snapshot.v2 {
            plan.scanned += 1;
            if snapshot.protected_run_ids.contains(&row.run_id) {
                plan.skipped.push(skip(
                    format!("v2:{}", row.id),
                    "retained_run",
                    format!("run {} still has retained job evidence", row.run_id),
                ));
            } else if row.ts < snapshot.cutoff {
                plan.candidates.push(candidate(
                    format!("v2:{}", row.id),
                    "delete_v2_event",
                    None,
                    format!("v2 row {} is scoped to {}", row.id, self.workspace_id),
                    format!("timestamp {} is before {}", row.ts, snapshot.cutoff),
                    row.event_id.clone(),
                ));
            }
        }
        for file in &snapshot.jsonl {
            plan.scanned += 1;
            if file.eligible {
                plan.candidates.push(candidate(
                    format!("jsonl:{}", file.path.display()),
                    "delete_loop_jsonl",
                    Some(file.path.clone()),
                    "file is beneath workspace state/audit/v2_loop".to_string(),
                    format!("every valid envelope predates {}", snapshot.cutoff),
                    file.fingerprint.clone(),
                ));
            } else {
                plan.skipped.push(skip(
                    format!("jsonl:{}", file.path.display()),
                    file.skip_code.as_deref().unwrap_or("retained_envelope"),
                    file.skip_reason
                        .as_deref()
                        .unwrap_or("contains retained evidence"),
                ));
            }
        }

        for blob in scan_blobs(&self.audit_root.join("blobs"))? {
            plan.scanned += 1;
            let hash = blob
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            let bytes = fs::symlink_metadata(&blob).ok().map(|meta| meta.len());
            if snapshot.retained_refs.contains(&hash) {
                plan.skipped.push(skip(format!("blob:{hash}"), "referenced", "blob is reachable from retained audit, a hold/export, or retained run evidence"));
            } else {
                let expected = file_fingerprint(&blob)?;
                plan.candidates.push(GcCandidate {
                    id: format!("blob:{hash}"),
                    action: "sweep_unreachable_blob".to_string(),
                    path: Some(blob),
                    bytes,
                    ownership_evidence: "content-addressed file beneath workspace audit blob root"
                        .to_string(),
                    retention_evidence:
                        "hash is absent from the complete retained-reference mark set".to_string(),
                    expected_state: expected,
                    allow_owned_symlink: false,
                });
            }
        }
        for hash in &snapshot.retained_refs {
            let path = self.audit_root.join("blobs").join(&hash[..2]).join(hash);
            if !path.exists() {
                plan.errors.push(GcItemError {
                    id: format!("blob:{hash}"),
                    phase: "scan".to_string(),
                    code: "missing_referenced_blob".to_string(),
                    message: "retained evidence references a blob that is already missing"
                        .to_string(),
                });
            }
        }
        plan.scanned_bytes = Some(plan.candidates.iter().filter_map(|item| item.bytes).sum());
        Ok(plan)
    }

    fn revalidate(
        &self,
        candidate: &GcCandidate,
        context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError> {
        if let Some(raw) = candidate.id.strip_prefix("legacy:") {
            let id = parse_id(raw)?;
            let cutoff = self.cutoff(context)?;
            let current = self
                .store
                .list_legacy_audit_rows_for_gc()?
                .into_iter()
                .find(|row| row.id == id);
            return Ok(match current {
                None => stale_skip("legacy row was already removed"),
                Some(row)
                    if row.timestamp.to_rfc3339() != candidate.expected_state
                        || row.timestamp >= cutoff =>
                {
                    stale_skip("legacy row changed or is no longer expired")
                }
                Some(_) => GcRevalidation::Ready,
            });
        }
        if let Some(raw) = candidate.id.strip_prefix("v2:") {
            let id = parse_id(raw)?;
            let cutoff = self.cutoff(context)?;
            let current = self
                .store
                .list_v2_audit_events_for_gc(&self.workspace_id)?
                .into_iter()
                .find(|row| row.id == id);
            return Ok(match current {
                None => stale_skip("v2 row was already removed"),
                Some(row) if row.event_id != candidate.expected_state || row.ts >= cutoff => {
                    stale_skip("v2 row changed or is no longer expired")
                }
                Some(row)
                    if protected_evidence(context.scope.root())?
                        .run_ids
                        .contains(&row.run_id) =>
                {
                    stale_skip("v2 row is now protected by retained run evidence")
                }
                Some(_) => GcRevalidation::Ready,
            });
        }
        let path = candidate
            .path
            .as_deref()
            .ok_or_else(|| OrbitError::Execution("audit file candidate has no path".to_string()))?;
        if !path.exists() {
            return Ok(stale_skip("file was already removed"));
        }
        if file_fingerprint(path)? != candidate.expected_state {
            return Ok(stale_skip("file changed after the plan was frozen"));
        }
        if let Some(hash) = candidate.id.strip_prefix("blob:")
            && self.blob_is_referenced(hash, context)?
        {
            return Ok(stale_skip("blob became reachable after planning"));
        }
        Ok(GcRevalidation::Ready)
    }

    fn apply(
        &self,
        candidate: &GcCandidate,
        context: &GcContext<'_>,
    ) -> Result<GcMutation, OrbitError> {
        // Audit writer/GC synchronization (ORB-10186). Acquire the
        // workspace-scoped audit writer guard — the *same* advisory lock every
        // audit writer path holds across its publication (workspace v2 event
        // publication, loop event/JSONL append, and content-addressed blob
        // publication) — and keep holding it while we (1) re-run the final
        // mark/fingerprint validation and (2) delete the envelope or blob. No
        // writer can slip a retained reference (or a JSONL append) between the
        // re-mark and the unlink. Lock ordering: `execute_gc` already holds the
        // host-global GC lock (ADR-0220); we take the audit guard beneath it,
        // then mutate the filesystem — GC host lock → audit writer guard →
        // filesystem. A concurrent writer contends on this same guard: if it
        // published first, the re-mark observes the new reference / changed
        // fingerprint and we skip (fail closed); if we hold first, the writer
        // blocks until deletion completes and then republishes (its blob write
        // recreates the content-addressed object) rather than stranding a
        // reference to a swept blob.
        let _writer_guard = audit_writer_guard::acquire(&self.audit_root)?;
        match self.revalidate(candidate, context)? {
            GcRevalidation::Ready => {}
            GcRevalidation::Skip { code, reason } => {
                return Err(OrbitError::Execution(format!(
                    "audit evidence changed between planning and deletion ({code}): {reason}"
                )));
            }
        }
        if let Some(raw) = candidate.id.strip_prefix("legacy:") {
            let timestamp = DateTime::parse_from_rfc3339(&candidate.expected_state)
                .map_err(|error| {
                    OrbitError::Execution(format!("invalid frozen timestamp: {error}"))
                })?
                .with_timezone(&Utc);
            let _ = self
                .store
                .delete_legacy_audit_row_for_gc(parse_id(raw)?, &timestamp)?;
            return Ok(GcMutation {
                reclaimed_bytes: None,
            });
        }
        if let Some(raw) = candidate.id.strip_prefix("v2:") {
            let _ = self.store.delete_v2_audit_event_for_gc(
                &self.workspace_id,
                parse_id(raw)?,
                &candidate.expected_state,
            )?;
            return Ok(GcMutation {
                reclaimed_bytes: None,
            });
        }
        let path = candidate
            .path
            .as_deref()
            .ok_or_else(|| OrbitError::Execution("audit file candidate has no path".to_string()))?;
        let bytes = fs::symlink_metadata(path)?.len();
        fs::remove_file(path)?;
        if candidate.id.starts_with("blob:")
            && let Some(parent) = path.parent()
            && fs::read_dir(parent).is_ok_and(|mut entries| entries.next().is_none())
        {
            let _ = fs::remove_dir(parent);
        }
        Ok(GcMutation {
            reclaimed_bytes: Some(bytes),
        })
    }
}

struct AuditSnapshot {
    cutoff: DateTime<Utc>,
    repo_root: PathBuf,
    legacy: Vec<AuditGcLegacyRow>,
    v2: Vec<V2AuditEventRow>,
    jsonl: Vec<JsonlFile>,
    retained_refs: HashSet<String>,
    protected_run_ids: HashSet<String>,
}

struct ProtectedEvidence {
    blob_refs: HashSet<String>,
    run_ids: HashSet<String>,
}

struct JsonlFile {
    path: PathBuf,
    fingerprint: String,
    eligible: bool,
    blob_refs: HashSet<String>,
    skip_code: Option<String>,
    skip_reason: Option<String>,
}

fn protected_evidence(orbit_root: &Path) -> Result<ProtectedEvidence, OrbitError> {
    let mut blob_refs = HashSet::new();
    let mut run_ids = HashSet::new();
    for root in [
        orbit_root.join("state/audit/holds"),
        orbit_root.join("state/audit/exports"),
        orbit_root.join("state/job-runs"),
    ] {
        for path in recursive_files(&root)? {
            if root.ends_with("job-runs") {
                for component in path
                    .components()
                    .filter_map(|part| part.as_os_str().to_str())
                {
                    if component.starts_with("jrun-") || component.starts_with("run-") {
                        run_ids.insert(component.to_string());
                    }
                }
            }
            if let Ok(contents) = fs::read_to_string(&path) {
                collect_blob_refs(&contents, &mut blob_refs);
                collect_run_ids(&contents, &mut run_ids);
            }
        }
    }
    Ok(ProtectedEvidence { blob_refs, run_ids })
}

fn scan_jsonl(
    root: &Path,
    cutoff: DateTime<Utc>,
    protected_runs: &HashSet<String>,
) -> Result<Vec<JsonlFile>, OrbitError> {
    let mut files = Vec::new();
    for path in recursive_files(root)? {
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let contents = fs::read_to_string(&path)?;
        let mut blob_refs = HashSet::new();
        let mut all_old = true;
        let mut malformed = false;
        let mut protected = false;
        let mut saw_event = false;
        for line in contents.lines().filter(|line| !line.trim().is_empty()) {
            collect_blob_refs(line, &mut blob_refs);
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                malformed = true;
                continue;
            };
            saw_event = true;
            let Some(ts) = event_timestamp(&value) else {
                malformed = true;
                continue;
            };
            all_old &= ts < cutoff;
            if value_string(&value, "run_id").is_some_and(|run| protected_runs.contains(run)) {
                protected = true;
            }
        }
        let (skip_code, skip_reason) = if malformed || !saw_event {
            (
                Some("malformed_jsonl".to_string()),
                Some(
                    "file contains malformed or timestamp-free JSONL and is retained fail-closed"
                        .to_string(),
                ),
            )
        } else if protected {
            (
                Some("retained_run".to_string()),
                Some("file belongs to a retained job run".to_string()),
            )
        } else if !all_old {
            (
                Some("retained_envelope".to_string()),
                Some("file contains an envelope inside retention".to_string()),
            )
        } else {
            (None, None)
        };
        files.push(JsonlFile {
            fingerprint: file_fingerprint(&path)?,
            path,
            eligible: all_old && !malformed && !protected && saw_event,
            blob_refs,
            skip_code,
            skip_reason,
        });
    }
    Ok(files)
}

fn scan_blobs(root: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    Ok(recursive_files(root)?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_blob_hash)
        })
        .collect())
}

fn recursive_files(root: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                files.push(entry.path());
            }
        }
    }
    Ok(files)
}

fn collect_legacy_refs(row: &AuditGcLegacyRow, refs: &mut HashSet<String>) {
    for value in [
        &row.arguments_json,
        &row.stdout_truncated,
        &row.stderr_truncated,
    ]
    .into_iter()
    .flatten()
    {
        collect_blob_refs(value, refs);
    }
}

fn collect_blob_refs(text: &str, refs: &mut HashSet<String>) {
    for token in text.split(|ch: char| !ch.is_ascii_hexdigit()) {
        if is_blob_hash(token) {
            refs.insert(token.to_ascii_lowercase());
        }
    }
}

fn collect_run_ids(text: &str, runs: &mut HashSet<String>) {
    for token in text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-')) {
        if token.starts_with("jrun-") || token.starts_with("run-") {
            runs.insert(token.to_string());
        }
    }
}

fn is_blob_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn event_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    value_string(value, "ts")
        .or_else(|| value_string(value, "timestamp"))
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|ts| ts.with_timezone(&Utc))
}

fn value_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .or_else(|| value.get("envelope")?.get(key)?.as_str())
}

fn working_directory_in_scope(raw: &str, repo_root: &Path) -> bool {
    Path::new(raw).starts_with(repo_root)
}

fn file_fingerprint(path: &Path) -> Result<String, OrbitError> {
    let metadata = fs::symlink_metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |value| value.as_nanos());
    Ok(format!("{}:{modified}", metadata.len()))
}

fn parse_duration_seconds(raw: &str) -> Result<i64, OrbitError> {
    let split = raw
        .find(|ch: char| ch.is_ascii_alphabetic())
        .ok_or_else(|| OrbitError::InvalidInput(format!("invalid retention duration `{raw}`")))?;
    let count: i64 = raw[..split]
        .parse()
        .map_err(|_| OrbitError::InvalidInput(format!("invalid retention duration `{raw}`")))?;
    let multiplier = match &raw[split..] {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "w" => 604_800,
        _ => {
            return Err(OrbitError::InvalidInput(format!(
                "invalid retention duration `{raw}`"
            )));
        }
    };
    count
        .checked_mul(multiplier)
        .ok_or_else(|| OrbitError::InvalidInput(format!("retention duration `{raw}` is too large")))
}

fn parse_id(raw: &str) -> Result<i64, OrbitError> {
    raw.parse()
        .map_err(|_| OrbitError::Execution(format!("invalid audit candidate id `{raw}`")))
}

fn candidate(
    id: String,
    action: &str,
    path: Option<PathBuf>,
    ownership: String,
    retention: String,
    expected_state: String,
) -> GcCandidate {
    GcCandidate {
        id,
        action: action.to_string(),
        path,
        bytes: None,
        ownership_evidence: ownership,
        retention_evidence: retention,
        expected_state,
        allow_owned_symlink: false,
    }
}

fn skip(id: String, code: &str, reason: impl Into<String>) -> GcSkip {
    GcSkip {
        id,
        code: code.to_string(),
        reason: reason.into(),
    }
}

fn stale_skip(reason: &str) -> GcRevalidation {
    GcRevalidation::Skip {
        code: "stale_plan".to_string(),
        reason: reason.to_string(),
    }
}
