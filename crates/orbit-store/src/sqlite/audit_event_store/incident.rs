//! Failure-incident grouping over raw audit events [ORB-10871].
//!
//! A raw failed audit event is forensic evidence, not an incident. One burst
//! of the same refusal repeated hundreds of times is one problem, and one
//! failed pipeline run that propagates its failure up through its enclosing
//! steps is one root cause with a chain hanging off it. Counting the raw rows
//! as independent failures overstates both.
//!
//! This module derives an incident view *on top of* the audit rows without
//! consuming or rewriting them: every incident carries its raw `event_count`
//! and the ids of the rows it collapsed, so the raw Audit view and any export
//! stay the authority on evidence.
//!
//! The contract is deliberately project-agnostic — it reads only durable
//! audit columns (status, role, tool/command, error message, run/task ids,
//! timestamps). No tool name, agent, workspace, or task id is special-cased.
//!
//! Grouping runs in three passes:
//!
//! 1. **Classify** each failed row as [`FailureClass::Denied`],
//!    [`FailureClass::Expected`], or [`FailureClass::Unexpected`] so a policy
//!    refusal and a caller's invalid input stay distinguishable from a genuine
//!    unexpected failure.
//! 2. **Cluster** rows by `(run scope, signature)`, where the signature is
//!    `class | role | surface | normalized message`. Volatile tokens (paths,
//!    numbers, ids, timestamps, hashes, quoted literals) are replaced with
//!    placeholders, so a repeated failure whose only difference is its operand
//!    collapses into one cluster.
//! 3. **Collapse cascades** within a single job run: clusters of the same
//!    class whose time ranges are within [`CASCADE_WINDOW_SECS`] of each other
//!    are one incident, rooted at the earliest cluster, with the later ones
//!    recorded as its propagation chain. A failure later in the same run,
//!    beyond that window, stays an independent incident.

use std::borrow::Cow;
use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use orbit_common::OrbitError;
use orbit_types::telemetry::{AuditEvent, AuditEventStatus};
use serde::{Deserialize, Serialize};

use super::{AUDIT_EVENT_COLUMNS, audit_event_from_row};
use crate::Store;

/// Maximum gap between one cluster's last event and the next cluster's first
/// event for the two to be treated as one cascade within a job run. Failure
/// propagation up a pipeline's enclosing steps is effectively immediate; a
/// minute of slack absorbs step teardown without swallowing a genuinely
/// separate failure later in the same run.
pub const CASCADE_WINDOW_SECS: i64 = 60;

/// Per-incident cap on the sampled raw events echoed back to callers. The
/// full population is always reachable through the raw Audit view; this bound
/// keeps one burst of thousands from dominating a response body.
const MAX_SAMPLE_EVENTS: usize = 20;

/// Longest normalized message retained in a signature. Long enough to keep
/// distinct failures distinct, short enough that a multi-kilobyte error body
/// cannot become a grouping key.
const MAX_SIGNATURE_MESSAGE_CHARS: usize = 160;

/// How an audit failure should be read by an operator.
///
/// The three classes are kept separate at every layer: an incident never
/// merges rows of different classes, so a policy denial can never be counted
/// as an unexpected failure (or hide one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// The call was refused by policy or capability before it ran.
    Denied,
    /// The call ran and failed on a documented negative path — invalid input,
    /// a missing record, a validation refusal. The system behaved correctly.
    Expected,
    /// Everything else: a failure that is not a known negative path.
    Unexpected,
}

impl FailureClass {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureClass::Denied => "denied",
            FailureClass::Expected => "expected",
            FailureClass::Unexpected => "unexpected",
        }
    }

    /// Operator-facing label. Rendered next to the count so "12 denied" is
    /// never mistaken for "12 things broke".
    pub fn label(self) -> &'static str {
        match self {
            FailureClass::Denied => "policy denial",
            FailureClass::Expected => "expected negative path",
            FailureClass::Unexpected => "unexpected failure",
        }
    }
}

/// Message fragments that mark a documented negative path. Each mirrors an
/// `OrbitError` variant's `Display` rendering in `orbit-common`, which every
/// surface funnels its errors through before they reach `error_message`.
/// Matching is lowercase substring, so a wrapped/prefixed message still
/// classifies.
const EXPECTED_FAILURE_MARKERS: &[&str] = &[
    "invalid input",
    "not found",
    "already exists",
    "validation failed",
    "sensitive input rejected",
    "invalid status transition",
    "unsupported",
    "not local to the current worktree",
    "artifact unavailable",
];

/// Message fragments that mark a refusal rather than a failure. A refusal
/// recorded with `status = failure` (some surfaces translate late) still
/// classifies as a denial so the two populations do not blur.
const DENIAL_MARKERS: &[&str] = &["policy denied", "capability denied", "permission denied"];

/// One raw audit row referenced by an incident. Enough to identify the exact
/// underlying evidence and jump to it in the raw Audit view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncidentEventRef {
    pub id: i64,
    pub ts: DateTime<Utc>,
    pub execution_id: String,
    pub status: String,
    pub role: String,
    pub surface: String,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub activity_id: Option<String>,
    pub message: Option<String>,
}

/// One downstream link of a cascade: a distinct failure signature that
/// followed the incident's root within the same run and cascade window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropagationLink {
    pub signature: String,
    /// Surface (tool name, or `command`/`subcommand`) that reported it.
    pub surface: String,
    /// Enclosing activity/step id when the audit row carried one.
    pub activity_id: Option<String>,
    pub event_count: u64,
    pub first_ts: DateTime<Utc>,
    pub last_ts: DateTime<Utc>,
    pub message: Option<String>,
    pub sample_events: Vec<IncidentEventRef>,
}

/// A grouped failure incident: one root cause, its raw event population, and
/// the propagation chain it triggered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureIncident {
    /// Deterministic id derived from the grouping key. Stable across repeated
    /// aggregation of the same rows; never a database identifier.
    pub incident_id: String,
    /// Human-readable grouping key. Shown in the UI so an operator can see
    /// *why* these rows were grouped.
    pub signature: String,
    pub class: FailureClass,
    pub role: String,
    pub surface: String,
    pub activity_id: Option<String>,
    pub message: Option<String>,
    /// Raw audit rows collapsed into this incident, including its
    /// propagation chain. This is the forensic count.
    pub event_count: u64,
    /// Raw audit rows matching the root signature alone.
    pub root_event_count: u64,
    pub first_ts: DateTime<Utc>,
    pub last_ts: DateTime<Utc>,
    pub run_ids: Vec<String>,
    pub task_ids: Vec<String>,
    /// Bounded sample of the root's raw rows, newest first.
    pub sample_events: Vec<IncidentEventRef>,
    /// Downstream failures collapsed beneath the root, in occurrence order.
    pub propagation: Vec<PropagationLink>,
}

impl FailureIncident {
    /// Raw rows attributed to the propagation chain only.
    pub fn propagated_event_count(&self) -> u64 {
        self.event_count.saturating_sub(self.root_event_count)
    }
}

/// Window and scope for a failure-incident aggregation.
#[derive(Debug, Clone, Default)]
pub struct FailureIncidentQuery {
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub role: Option<String>,
    pub workspace_id: Option<String>,
    /// Cap on raw rows scanned. `0` applies [`DEFAULT_SCAN_LIMIT`].
    pub max_events: usize,
}

/// Default cap on raw failure rows scanned for one aggregation.
pub const DEFAULT_SCAN_LIMIT: usize = 20_000;

/// Result of one aggregation: the incidents plus the raw denominators they
/// were derived from, so no surface has to re-derive (or guess) them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureIncidentReport {
    pub incidents: Vec<FailureIncident>,
    /// Raw failed audit rows considered (all classes).
    pub raw_failed_events: u64,
    /// Raw rows per class, keyed by [`FailureClass::as_str`].
    pub raw_events_by_class: BTreeMap<String, u64>,
    /// Incident count per class.
    pub incidents_by_class: BTreeMap<String, u64>,
    /// True when the scan hit `max_events` and older rows were not read.
    pub truncated: bool,
}

impl FailureIncidentReport {
    pub fn incident_count(&self) -> u64 {
        self.incidents.len() as u64
    }
}

impl Store {
    /// Failure incidents over the audit rows matching `query`.
    ///
    /// Reads only `status IN ('failure', 'denied')` rows; success rows are
    /// never touched, and no row is mutated or hidden.
    pub fn get_failure_incidents(
        &self,
        query: &FailureIncidentQuery,
    ) -> Result<FailureIncidentReport, OrbitError> {
        let max_events = if query.max_events == 0 {
            DEFAULT_SCAN_LIMIT
        } else {
            query.max_events
        };

        let conn = self.read()?;
        let mut conditions = vec!["status != 'success'".to_string()];
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(since) = query.since {
            conditions.push(format!("timestamp >= ?{}", param_values.len() + 1));
            param_values.push(Box::new(since.to_rfc3339()));
        }
        if let Some(until) = query.until {
            conditions.push(format!("timestamp <= ?{}", param_values.len() + 1));
            param_values.push(Box::new(until.to_rfc3339()));
        }
        if let Some(role) = query.role.as_deref().filter(|role| !role.is_empty()) {
            conditions.push(format!("role = ?{}", param_values.len() + 1));
            param_values.push(Box::new(role.to_string()));
        }
        if let Some(workspace_id) = query
            .workspace_id
            .as_deref()
            .filter(|workspace_id| !workspace_id.is_empty())
        {
            conditions.push(format!("workspace_id = ?{}", param_values.len() + 1));
            param_values.push(Box::new(workspace_id.to_string()));
        }

        let sql = format!(
            "SELECT {AUDIT_EVENT_COLUMNS} FROM audit_events WHERE {} \
             ORDER BY id DESC LIMIT ?{}",
            conditions.join(" AND "),
            param_values.len() + 1
        );
        param_values.push(Box::new(max_events as i64));

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|value| value.as_ref()).collect();
        let failures = stmt
            .query_map(param_refs.as_slice(), audit_event_from_row)
            .map_err(|e| OrbitError::Store(e.to_string()))?
            .collect::<Result<Vec<AuditEvent>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let truncated = failures.len() >= max_events;
        Ok(build_report(&failures, truncated))
    }
}

/// Groups already-filtered failure rows into a report. Split out from the
/// store call so the contract is testable without a database.
pub fn build_report(failures: &[AuditEvent], truncated: bool) -> FailureIncidentReport {
    let incidents = group_failure_incidents(failures);

    let mut raw_events_by_class: BTreeMap<String, u64> = BTreeMap::new();
    for event in failures {
        *raw_events_by_class
            .entry(classify(event).as_str().to_string())
            .or_insert(0) += 1;
    }
    let mut incidents_by_class: BTreeMap<String, u64> = BTreeMap::new();
    for incident in &incidents {
        *incidents_by_class
            .entry(incident.class.as_str().to_string())
            .or_insert(0) += 1;
    }

    FailureIncidentReport {
        raw_failed_events: failures.len() as u64,
        raw_events_by_class,
        incidents_by_class,
        incidents,
        truncated,
    }
}

/// Classifies one failed audit row. Denial status wins outright; otherwise the
/// `OrbitError`-derived markers decide whether this was a documented negative
/// path. Anything unmatched is treated as unexpected — the conservative
/// direction, since under-reporting a real failure is the costlier mistake.
pub fn classify(event: &AuditEvent) -> FailureClass {
    if matches!(event.status, AuditEventStatus::Denied) {
        return FailureClass::Denied;
    }
    let message = event
        .error_message
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    if message.is_empty() {
        return FailureClass::Unexpected;
    }
    if DENIAL_MARKERS.iter().any(|marker| message.contains(marker)) {
        return FailureClass::Denied;
    }
    if EXPECTED_FAILURE_MARKERS
        .iter()
        .any(|marker| message.contains(marker))
    {
        return FailureClass::Expected;
    }
    FailureClass::Unexpected
}

/// The reporting surface of an audit row: its tool name when it has one,
/// otherwise its `command`/`subcommand` pair. Never empty.
fn surface_of(event: &AuditEvent) -> String {
    if let Some(tool) = event.tool_name.as_deref().filter(|name| !name.is_empty()) {
        return tool.to_string();
    }
    match event.subcommand.as_deref().filter(|sub| !sub.is_empty()) {
        Some(sub) if !event.command.is_empty() => format!("{} {sub}", event.command),
        Some(sub) => sub.to_string(),
        None if !event.command.is_empty() => event.command.clone(),
        None => "unknown".to_string(),
    }
}

/// Replaces volatile tokens in an error message so that the same failure over
/// different operands collapses to one signature.
///
/// The rules are shape-based only — no vocabulary from any project, tool, or
/// agent appears here.
pub fn normalize_message(raw: &str) -> String {
    let mut out = String::new();
    for token in raw.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&normalize_token(token));
        if out.chars().count() >= MAX_SIGNATURE_MESSAGE_CHARS {
            break;
        }
    }
    if out.chars().count() > MAX_SIGNATURE_MESSAGE_CHARS {
        out = out.chars().take(MAX_SIGNATURE_MESSAGE_CHARS).collect();
    }
    out
}

fn normalize_token(token: &str) -> Cow<'_, str> {
    let trimmed = token.trim_matches(['"', '\'', '`', '(', ')', ',']);
    if trimmed.is_empty() {
        return Cow::Borrowed(token);
    }
    // Order matters: the most specific shapes are tested first so a token that
    // satisfies several rules gets its most informative placeholder.
    if looks_like_timestamp(trimmed) {
        return Cow::Borrowed("<ts>");
    }
    if looks_like_uuid(trimmed) {
        return Cow::Borrowed("<uuid>");
    }
    if looks_like_prefixed_id(trimmed) {
        return Cow::Borrowed("<id>");
    }
    if looks_like_path(trimmed) {
        return Cow::Borrowed("<path>");
    }
    if looks_like_hex(trimmed) {
        return Cow::Borrowed("<hex>");
    }
    if looks_like_number(trimmed) {
        return Cow::Borrowed("<num>");
    }
    Cow::Owned(trimmed.to_string())
}

/// `2026-08-15T06:34:00Z` and friends: leading four digits, a `-`, and a
/// digit-dominated body.
fn looks_like_timestamp(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() >= 10
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && token.chars().filter(char::is_ascii_digit).count() >= 8
}

fn looks_like_uuid(token: &str) -> bool {
    let groups: Vec<&str> = token.split('-').collect();
    groups.len() == 5
        && [8_usize, 4, 4, 4, 12]
            .iter()
            .zip(&groups)
            .all(|(len, group)| group.len() == *len && group.chars().all(|c| c.is_ascii_hexdigit()))
}

/// `ABC-1234`, `T20260428-7`, `jrun-20260816-0634-8`: an alphanumeric stem
/// joined to a digit run. Covers generated record ids of any project without
/// naming one.
fn looks_like_prefixed_id(token: &str) -> bool {
    let Some((head, tail)) = token.split_once('-') else {
        return false;
    };
    if head.is_empty() || !head.chars().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    if !head.chars().any(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let rest: String = tail.chars().filter(|c| *c != '-').collect();
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

fn looks_like_path(token: &str) -> bool {
    token.contains('/') || token.starts_with('~') || token.contains('\\')
}

fn looks_like_hex(token: &str) -> bool {
    token.len() >= 8
        && token.chars().all(|c| c.is_ascii_hexdigit())
        && token.chars().any(|c| c.is_ascii_alphabetic())
}

fn looks_like_number(token: &str) -> bool {
    let stripped = token.trim_end_matches(['.', ':', ';']);
    !stripped.is_empty()
        && stripped
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '_' | '-' | '+'))
        && stripped.chars().any(|c| c.is_ascii_digit())
}

/// The grouping signature for one failed row: class, actor, surface, and the
/// normalized message. Rendered as a readable string so the UI can show the
/// operator exactly what was collapsed.
pub fn signature_for(event: &AuditEvent) -> String {
    let class = classify(event);
    let message = normalize_message(event.error_message.as_deref().unwrap_or_default());
    format!(
        "{}|role={}|surface={}|msg={}",
        class.as_str(),
        if event.role.is_empty() {
            "unknown"
        } else {
            event.role.as_str()
        },
        surface_of(event),
        if message.is_empty() {
            format!("exit={}", event.exit_code)
        } else {
            message
        }
    )
}

/// Deterministic 64-bit FNV-1a over the grouping key, rendered as hex. Used
/// only as a stable client-side handle for an incident — never persisted, and
/// never a security boundary.
fn incident_id(run_scope: &str, signature: &str, first_ts: DateTime<Utc>) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let key = format!(
        "{run_scope}\u{1f}{signature}\u{1f}{}",
        first_ts.to_rfc3339()
    );
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("inc-{hash:016x}")
}

/// One `(run scope, signature)` bucket before cascade collapsing.
#[derive(Debug, Clone)]
struct Cluster {
    signature: String,
    class: FailureClass,
    role: String,
    surface: String,
    activity_id: Option<String>,
    message: Option<String>,
    event_count: u64,
    first_ts: DateTime<Utc>,
    last_ts: DateTime<Utc>,
    run_ids: Vec<String>,
    task_ids: Vec<String>,
    sample_events: Vec<IncidentEventRef>,
}

impl Cluster {
    fn absorb(&mut self, event: &AuditEvent) {
        self.event_count += 1;
        self.first_ts = self.first_ts.min(event.timestamp);
        self.last_ts = self.last_ts.max(event.timestamp);
        if self.activity_id.is_none() {
            self.activity_id.clone_from(&event.activity_id);
        }
        if self.message.is_none() {
            self.message.clone_from(&event.error_message);
        }
        push_unique(&mut self.run_ids, event.job_run_id.as_deref());
        push_unique(&mut self.task_ids, event.task_id.as_deref());
        if self.sample_events.len() < MAX_SAMPLE_EVENTS {
            self.sample_events.push(event_ref(event));
        }
    }

    fn into_link(mut self) -> PropagationLink {
        sort_samples(&mut self.sample_events);
        PropagationLink {
            signature: self.signature,
            surface: self.surface,
            activity_id: self.activity_id,
            event_count: self.event_count,
            first_ts: self.first_ts,
            last_ts: self.last_ts,
            message: self.message,
            sample_events: self.sample_events,
        }
    }
}

fn push_unique(target: &mut Vec<String>, value: Option<&str>) {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return;
    };
    if !target.iter().any(|existing| existing == value) {
        target.push(value.to_string());
    }
}

fn event_ref(event: &AuditEvent) -> IncidentEventRef {
    IncidentEventRef {
        id: event.id,
        ts: event.timestamp,
        execution_id: event.execution_id.clone(),
        status: event.status.to_string(),
        role: event.role.clone(),
        surface: surface_of(event),
        run_id: event.job_run_id.clone(),
        task_id: event.task_id.clone(),
        activity_id: event.activity_id.clone(),
        message: event.error_message.clone(),
    }
}

/// Newest raw evidence first, ties broken by id so a page is stable.
fn sort_samples(samples: &mut [IncidentEventRef]) {
    samples.sort_by(|a, b| b.ts.cmp(&a.ts).then_with(|| b.id.cmp(&a.id)));
}

/// Groups failed audit rows into incidents. Pure and order-independent: the
/// same rows in any input order produce the same incidents.
pub fn group_failure_incidents(failures: &[AuditEvent]) -> Vec<FailureIncident> {
    // Pass 1+2 — cluster by (run scope, signature).
    let mut clusters: BTreeMap<(String, String), Cluster> = BTreeMap::new();
    for event in failures {
        let signature = signature_for(event);
        let run_scope = event
            .job_run_id
            .clone()
            .filter(|run| !run.is_empty())
            .unwrap_or_default();
        clusters
            .entry((run_scope, signature.clone()))
            .and_modify(|cluster| cluster.absorb(event))
            .or_insert_with(|| {
                let mut cluster = Cluster {
                    signature,
                    class: classify(event),
                    role: event.role.clone(),
                    surface: surface_of(event),
                    activity_id: None,
                    message: None,
                    event_count: 0,
                    first_ts: event.timestamp,
                    last_ts: event.timestamp,
                    run_ids: Vec::new(),
                    task_ids: Vec::new(),
                    sample_events: Vec::new(),
                };
                cluster.absorb(event);
                cluster
            });
    }

    // Pass 3 — collapse same-run, same-class cascades onto their earliest
    // cluster. Clusters with no run id cannot be attributed to a pipeline, so
    // each stays its own incident (still deduplicated by signature above).
    let mut by_scope: BTreeMap<String, Vec<Cluster>> = BTreeMap::new();
    let mut incidents = Vec::new();
    for ((run_scope, _), cluster) in clusters {
        if run_scope.is_empty() {
            incidents.push(incident_from(&run_scope, cluster, Vec::new()));
        } else {
            by_scope.entry(run_scope).or_default().push(cluster);
        }
    }

    let cascade_window = Duration::seconds(CASCADE_WINDOW_SECS);
    for (run_scope, scope_clusters) in by_scope {
        for (_, mut class_clusters) in partition_by_class(scope_clusters) {
            class_clusters.sort_by(|a, b| {
                a.first_ts
                    .cmp(&b.first_ts)
                    .then_with(|| a.signature.cmp(&b.signature))
            });
            let mut iter = class_clusters.into_iter();
            let Some(mut root) = iter.next() else {
                continue;
            };
            let mut chain: Vec<Cluster> = Vec::new();
            let mut chain_end = root.last_ts;
            for cluster in iter {
                if cluster.first_ts - chain_end <= cascade_window {
                    chain_end = chain_end.max(cluster.last_ts);
                    chain.push(cluster);
                    continue;
                }
                incidents.push(incident_from(&run_scope, root, std::mem::take(&mut chain)));
                chain_end = cluster.last_ts;
                root = cluster;
            }
            incidents.push(incident_from(&run_scope, root, chain));
        }
    }

    // Newest incident first; the deterministic id breaks ties so two incidents
    // sharing a last-seen timestamp never swap places between renders.
    incidents.sort_by(|a, b| {
        b.last_ts
            .cmp(&a.last_ts)
            .then_with(|| a.incident_id.cmp(&b.incident_id))
    });
    incidents
}

fn partition_by_class(clusters: Vec<Cluster>) -> BTreeMap<FailureClass, Vec<Cluster>> {
    let mut out: BTreeMap<FailureClass, Vec<Cluster>> = BTreeMap::new();
    for cluster in clusters {
        out.entry(cluster.class).or_default().push(cluster);
    }
    out
}

fn incident_from(run_scope: &str, root: Cluster, chain: Vec<Cluster>) -> FailureIncident {
    let mut root = root;
    sort_samples(&mut root.sample_events);
    let mut event_count = root.event_count;
    let mut last_ts = root.last_ts;
    let mut run_ids = root.run_ids.clone();
    let mut task_ids = root.task_ids.clone();

    let mut propagation: Vec<PropagationLink> = Vec::with_capacity(chain.len());
    for cluster in chain {
        event_count += cluster.event_count;
        last_ts = last_ts.max(cluster.last_ts);
        for run_id in &cluster.run_ids {
            push_unique(&mut run_ids, Some(run_id));
        }
        for task_id in &cluster.task_ids {
            push_unique(&mut task_ids, Some(task_id));
        }
        propagation.push(cluster.into_link());
    }
    propagation.sort_by(|a, b| {
        a.first_ts
            .cmp(&b.first_ts)
            .then_with(|| a.signature.cmp(&b.signature))
    });

    FailureIncident {
        incident_id: incident_id(run_scope, &root.signature, root.first_ts),
        signature: root.signature,
        class: root.class,
        role: root.role,
        surface: root.surface,
        activity_id: root.activity_id,
        message: root.message,
        event_count,
        root_event_count: root.event_count,
        first_ts: root.first_ts,
        last_ts,
        run_ids,
        task_ids,
        sample_events: root.sample_events,
        propagation,
    }
}
